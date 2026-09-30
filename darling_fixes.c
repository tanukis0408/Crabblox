/* Workarounds for Darling runtime bugs that are not specific to one API.
 *
 * pthread_mutex_lock: Darling implements contended waits through darlingserver
 * (psynch_mutexwait) and can lose the wakeup, so a thread sleeps forever on a
 * mutex that is already free. Observed: the RakNet receive thread stuck in
 * std::mutex::lock with its socket queue full, game disconnected after 20 s.
 * Acquire with trylock plus yield/short sleeps first; only a mutex that stays
 * busy for ~200 ms falls back to the real (psynch) wait. A recursive mutex
 * already held by this thread succeeds in trylock, as with a real lock. */
extern int pthread_mutex_lock(void *);
extern int pthread_mutex_trylock(void *);
extern int sched_yield(void);
extern int usleep(unsigned int);
extern char *getenv(const char *);

#define DYLD_INTERPOSE(_replacement, _replacee) \
    __attribute__((used)) static struct { const void *replacement; const void *replacee; } \
    _interpose_##_replacee __attribute__((section("__DATA,__interpose"))) = \
        {(const void *)(unsigned long)&_replacement, (const void *)(unsigned long)&_replacee};

#define DARWIN_EBUSY 16

static int native_mutex(void) {
    static int native = -1;
    if (native < 0) {
        const char *value = getenv("MACOBLOX_NATIVE_MUTEX");
        native = value && value[0] ? 1 : 0;
    }
    return native;
}

static int macoblox_pthread_mutex_lock(void *mutex) {
    if (native_mutex())
        return pthread_mutex_lock(mutex);
    int result = pthread_mutex_trylock(mutex);
    if (result != DARWIN_EBUSY)
        return result;

    /* 1. Fast path: CPU pause instruction with exponential backoff.
     * Most critical sections in Roblox (physics, audio, RakNet, rendering) are
     * held for < 1 us. Spinning with __builtin_ia32_pause() resolves contentions
     * in user space with zero kernel syscalls or thread context switches. */
    for (int backoff = 1; backoff <= 32; backoff++) {
        for (int p = 0; p < backoff; p++) {
            __builtin_ia32_pause();
        }
        result = pthread_mutex_trylock(mutex);
        if (result != DARWIN_EBUSY)
            return result;
    }

    /* 2. Cooperative thread yields for medium-duration holds */
    for (int attempt = 0; attempt < 32; attempt++) {
        sched_yield();
        result = pthread_mutex_trylock(mutex);
        if (result != DARWIN_EBUSY)
            return result;
    }

    /* 3. Short backoff naps (max ~10 ms total) before falling back to darlingserver psynch wait */
    for (int attempt = 0; attempt < 64; attempt++) {
        usleep(attempt < 16 ? 50 : 150);
        result = pthread_mutex_trylock(mutex);
        if (result != DARWIN_EBUSY)
            return result;
    }
    return pthread_mutex_lock(mutex);
}
DYLD_INTERPOSE(macoblox_pthread_mutex_lock, pthread_mutex_lock)

/* Thread stacks: every Darling system call goes through an RPC to
 * darlingserver, which needs much more stack than the macOS kernel call it
 * replaces. FMOD creates its audio threads with small stacks that are enough
 * on macOS; under Darling one overflowed inside the RPC code
 * (dserver_rpc_hooks_receive_message, stack pointer just below its last
 * page) a few seconds after a game with sound started. Raise small requested
 * stack sizes to 1 MB. Threads with a caller-provided stack are left alone. */
typedef struct { long opaque[8]; } darwin_pthread_attr_t; /* 64 bytes on x86_64 */
extern int pthread_attr_setstacksize(darwin_pthread_attr_t *, unsigned long);
extern int pthread_attr_getstacksize(const darwin_pthread_attr_t *, unsigned long *);
extern int pthread_attr_getstackaddr(const darwin_pthread_attr_t *, void **);
extern int pthread_create(void **, const darwin_pthread_attr_t *, void *(*)(void *), void *);

#define MIN_THREAD_STACK (2UL << 20)

static int macoblox_pthread_attr_setstacksize(darwin_pthread_attr_t *attr, unsigned long size) {
    if (size < MIN_THREAD_STACK)
        size = MIN_THREAD_STACK;
    return pthread_attr_setstacksize(attr, size);
}
DYLD_INTERPOSE(macoblox_pthread_attr_setstacksize, pthread_attr_setstacksize)

static int macoblox_pthread_create(void **thread, const darwin_pthread_attr_t *attr,
                                   void *(*start)(void *), void *argument) {
    if (attr) {
        void *address = 0;
        unsigned long size = 0;
        if (pthread_attr_getstackaddr(attr, &address) == 0 && !address &&
            pthread_attr_getstacksize(attr, &size) == 0 && size < MIN_THREAD_STACK)
            pthread_attr_setstacksize((darwin_pthread_attr_t *)attr, MIN_THREAD_STACK);
    }
    return pthread_create(thread, attr, start, argument);
}
DYLD_INTERPOSE(macoblox_pthread_create, pthread_create)

/* Condition variables: the same lost psynch wakeups hit pthread_cond_wait.
 * Leaving a game, the network thread waited 9.4 s for a signal that had
 * already been sent (until its own ~10 s timeout). POSIX allows spurious
 * wakeups and correct callers re-check their predicate, so timed waits are
 * cut into slices that return as spurious wakeups. Every wait is an RPC to
 * darlingserver, so the slice starts at 50 ms and doubles up to 1 s while
 * the same thread keeps waiting on the same condition (flat 50 ms slices
 * cost darlingserver half a core). MACOBLOX_NATIVE_COND=1 turns this off. */
struct darwin_timespec { long tv_sec; long tv_nsec; };
struct darwin_timeval { long tv_sec; int tv_usec; };
extern int pthread_cond_wait(void *, void *);
extern int pthread_cond_timedwait(void *, void *, const struct darwin_timespec *);
extern int gettimeofday(struct darwin_timeval *, void *);

#define DARWIN_ETIMEDOUT 60
#define COND_SLICE_NS 50000000L

static int native_cond(void) {
    static int native = -1;
    if (native < 0) {
        const char *value = getenv("MACOBLOX_NATIVE_COND");
        native = value && value[0] ? 1 : 0;
    }
    return native;
}

#define COND_SLICE_MAX_NS 1000000000L

/* Per-thread slice state in pthread TSD slots: __thread variables would
 * go through dyld's TLV code, which itself waits on locks during startup
 * and deadlocked the client right after NSApplicationMain. */
extern int pthread_key_create(unsigned long *, void (*)(void *));
extern void *pthread_getspecific(unsigned long);
extern int pthread_setspecific(unsigned long, const void *);
static unsigned long cond_key, slice_key;
static volatile int keys_ready;

__attribute__((constructor)) static void create_slice_keys(void) {
    if (pthread_key_create(&cond_key, 0) == 0 && pthread_key_create(&slice_key, 0) == 0)
        keys_ready = 1;
}

/* The slice for this wait: doubles while the thread keeps timing out on the
 * same condition variable, starts again at 50 ms otherwise. */
static long next_slice(void *cond) {
    if (!keys_ready)
        return COND_SLICE_NS;
    long length = (long)pthread_getspecific(slice_key);
    if (cond != pthread_getspecific(cond_key) || !length) {
        pthread_setspecific(cond_key, cond);
        length = COND_SLICE_NS;
    } else if (length < COND_SLICE_MAX_NS) {
        length *= 2;
    }
    pthread_setspecific(slice_key, (void *)length);
    return length;
}

static void wait_finished(int sliced_out) {
    if (!sliced_out && keys_ready)
        pthread_setspecific(slice_key, 0);
}

static struct darwin_timespec slice_deadline_ns(long length) {
    struct darwin_timeval now;
    gettimeofday(&now, 0);
    struct darwin_timespec deadline = {now.tv_sec, now.tv_usec * 1000L + length};
    while (deadline.tv_nsec >= 1000000000L) {
        deadline.tv_sec++;
        deadline.tv_nsec -= 1000000000L;
    }
    return deadline;
}

/* pthread_cond_wait (no timeout) is left alone: code waiting without a
 * timeout is the most likely to skip re-checking its condition, and a
 * spurious wakeup there looked like a use-after-free (a garbage block on
 * the main dispatch queue crashed the client on the home page). Waits with
 * a timeout must cope with waking early anyway; the 9.4 s stall when
 * leaving a game was one of those. */

static int macoblox_pthread_cond_timedwait(void *cond, void *mutex, const struct darwin_timespec *abstime) {
    if (native_cond() || !abstime)
        return pthread_cond_timedwait(cond, mutex, abstime);
    struct darwin_timespec slice = slice_deadline_ns(next_slice(cond));
    int caller_deadline_first = abstime->tv_sec < slice.tv_sec ||
        (abstime->tv_sec == slice.tv_sec && abstime->tv_nsec <= slice.tv_nsec);
    if (caller_deadline_first) {
        wait_finished(0);
        return pthread_cond_timedwait(cond, mutex, abstime);
    }
    int result = pthread_cond_timedwait(cond, mutex, &slice);
    wait_finished(result == DARWIN_ETIMEDOUT);
    return result == DARWIN_ETIMEDOUT ? 0 : result;
}
DYLD_INTERPOSE(macoblox_pthread_cond_timedwait, pthread_cond_timedwait)

extern int pthread_cond_timedwait_relative_np(void *, void *, const struct darwin_timespec *);

static int macoblox_pthread_cond_timedwait_relative_np(void *cond, void *mutex,
                                                       const struct darwin_timespec *relative) {
    if (native_cond() || !relative)
        return pthread_cond_timedwait_relative_np(cond, mutex, relative);
    long length = next_slice(cond);
    if (relative->tv_sec * 1000000000L + relative->tv_nsec <= length) {
        wait_finished(0);
        return pthread_cond_timedwait_relative_np(cond, mutex, relative);
    }
    struct darwin_timespec slice = {length / 1000000000L, length % 1000000000L};
    int result = pthread_cond_timedwait_relative_np(cond, mutex, &slice);
    wait_finished(result == DARWIN_ETIMEDOUT);
    return result == DARWIN_ETIMEDOUT ? 0 : result;
}
DYLD_INTERPOSE(macoblox_pthread_cond_timedwait_relative_np, pthread_cond_timedwait_relative_np)

/* _availability_version_check (libxpc) backs every `@available(macOS ...)`
 * check. Darling's is a stub that returns false and logs "not implemented"
 * through os_log on every call; Roblox checks availability many times per
 * frame, which cost ~4% of the main thread in logging. Same answer, no log.
 * (Answering truthfully could enable code paths for APIs Darling lacks.) */
typedef struct { unsigned int platform, version; } darwin_build_version_t;
extern _Bool _availability_version_check(unsigned long, darwin_build_version_t *);

static _Bool macoblox_availability_version_check(unsigned long count, darwin_build_version_t *versions) {
    (void)count;
    (void)versions;
    return 0;
}
DYLD_INTERPOSE(macoblox_availability_version_check, _availability_version_check)

/* getpwuid / getpwnam: In Darling without opendirectoryd, getpwuid returns NULL.
 * RobloxCrashHandler and RobloxPlayer dereference pw_dir without checking for NULL,
 * crashing with SIGSEGV in RBX::FileSystem::getUserDirectory. */
struct darwin_passwd {
    char *pw_name;
    char *pw_passwd;
    int pw_uid;
    int pw_gid;
    long pw_change;
    char *pw_class;
    char *pw_gecos;
    char *pw_dir;
    char *pw_shell;
    long pw_expire;
};
extern struct darwin_passwd *getpwuid(unsigned int);
extern struct darwin_passwd *getpwnam(const char *);
extern int snprintf(char *, unsigned long, const char *, ...);

static char macoblox_default_home[256] = "/Users/user";
static char macoblox_default_user[64] = "user";

static struct darwin_passwd macoblox_fake_pw = {
    macoblox_default_user,
    "*",
    1000,
    1000,
    0,
    "",
    "User",
    macoblox_default_home,
    "/bin/bash",
    0,
};

static volatile int macoblox_pw_inited = 0;

static void macoblox_setup_fake_pw(const char *name_override, unsigned int uid) {
    if (!__sync_bool_compare_and_swap(&macoblox_pw_inited, 0, 1))
        return;
    const char *user = name_override && name_override[0] ? name_override : getenv("USER");
    const char *home = getenv("HOME");
    if (user && user[0]) {
        snprintf(macoblox_default_user, sizeof(macoblox_default_user), "%s", user);
        macoblox_fake_pw.pw_name = macoblox_default_user;
        macoblox_fake_pw.pw_gecos = macoblox_default_user;
    }
    if (home && home[0]) {
        macoblox_fake_pw.pw_dir = (char *)home;
    } else if (user && user[0]) {
        snprintf(macoblox_default_home, sizeof(macoblox_default_home), "/Users/%s", user);
        macoblox_fake_pw.pw_dir = macoblox_default_home;
    }
    macoblox_fake_pw.pw_uid = (int)uid;
}

static struct darwin_passwd *macoblox_getpwuid(unsigned int uid) {
    struct darwin_passwd *res = getpwuid(uid);
    if (res)
        return res;
    macoblox_setup_fake_pw(0, uid);
    return &macoblox_fake_pw;
}
DYLD_INTERPOSE(macoblox_getpwuid, getpwuid)

static struct darwin_passwd *macoblox_getpwnam(const char *name) {
    struct darwin_passwd *res = getpwnam(name);
    if (res)
        return res;
    macoblox_setup_fake_pw(name, 1000);
    return &macoblox_fake_pw;
}
DYLD_INTERPOSE(macoblox_getpwnam, getpwnam)

/* proc_pidinfo: Darling's libsystem_kernel lacks support for flavor 1 (PROC_PIDTASKINFO),
 * printing "sys_proc_info(): Unsupported pidinfo flavor: 1" and failing.
 * Roblox queries this to measure memory usage and resident set size.
 * Interpose flavor 1 and populate with real memory statistics from /proc/self/statm. */
#define DARWIN_PROC_PIDTASKINFO 1
struct darwin_proc_taskinfo {
    unsigned long long pti_virtual_size;       /* virtual memory size (bytes) */
    unsigned long long pti_resident_size;      /* resident memory size (bytes) */
    unsigned long long pti_total_user;         /* total user time */
    unsigned long long pti_total_system;       /* total system time */
    unsigned long long pti_threads_user;       /* total thread user time */
    unsigned long long pti_threads_system;     /* total thread system time */
    int                pti_policy;             /* default policy for new threads */
    int                pti_faults;             /* number of page faults */
    int                pti_pageins;            /* number of actual pageins */
    int                pti_cow_faults;         /* number of copy-on-write faults */
    int                pti_messages_sent;      /* number of messages sent */
    int                pti_messages_received;  /* number of messages received */
    int                pti_syscalls_mach;      /* number of mach system calls */
    int                pti_syscalls_unix;      /* number of unix system calls */
    int                pti_csw;                /* number of context switches */
    int                pti_threadnum;          /* number of threads in the task */
    int                pti_numrunning;         /* number of running threads */
    int                pti_priority;           /* task priority */
};

extern int proc_pidinfo(int, int, unsigned long long, void *, int);
extern int open(const char *, int, ...);
extern long read(int, void *, unsigned long);
extern int close(int);

static int macoblox_proc_pidinfo(int pid, int flavor, unsigned long long arg, void *buffer, int buffersize) {
    if (flavor == DARWIN_PROC_PIDTASKINFO && buffer && buffersize >= (int)sizeof(struct darwin_proc_taskinfo)) {
        struct darwin_proc_taskinfo *info = (struct darwin_proc_taskinfo *)buffer;
        unsigned char *p = (unsigned char *)buffer;
        for (unsigned long i = 0; i < sizeof(struct darwin_proc_taskinfo); i++) {
            p[i] = 0;
        }

        unsigned long long total_pages = 250000;
        unsigned long long res_pages = 100000;

        char path[64];
        if (pid <= 0) {
            snprintf(path, sizeof(path), "/proc/self/statm");
        } else {
            snprintf(path, sizeof(path), "/proc/%d/statm", pid);
        }

        int fd = open(path, 0 /* O_RDONLY */);
        if (fd >= 0) {
            char buf[128];
            long n = read(fd, buf, sizeof(buf) - 1);
            close(fd);
            if (n > 0) {
                buf[n] = 0;
                unsigned long long v1 = 0, v2 = 0;
                char *s = buf;
                while (*s == ' ' || *s == '\t') s++;
                while (*s >= '0' && *s <= '9') { v1 = v1 * 10 + (*s - '0'); s++; }
                while (*s == ' ' || *s == '\t') s++;
                while (*s >= '0' && *s <= '9') { v2 = v2 * 10 + (*s - '0'); s++; }
                if (v1 > 0) total_pages = v1;
                if (v2 > 0) res_pages = v2;
            }
        }

        info->pti_virtual_size = total_pages * 4096ULL;
        info->pti_resident_size = res_pages * 4096ULL;
        info->pti_threadnum = 16;
        info->pti_numrunning = 4;
        return (int)sizeof(struct darwin_proc_taskinfo);
    }
    return proc_pidinfo(pid, flavor, arg, buffer, buffersize);
}
DYLD_INTERPOSE(macoblox_proc_pidinfo, proc_pidinfo)


