/* Minimal X11 client: hides/shows the cursor with XFixes, and answers which
 * visual a window and the screen use (for picking EGL configs, gl_profile.c).
 *
 * Why: under Xwayland, XWarpPointer moves only the X server's pointer and the
 * next Wayland motion undoes it, so recentering during mouse lock bounced the
 * pointer. Xwayland emulates warps properly (locked pointer + relative motion)
 * while the X cursor is hidden with XFixes, as Wine does. Darling wraps
 * libX11 but not libXfixes, and sending the requests through Xlib internals
 * is unsafe (_XGetRequest/_XReply need Xlib's internal display lock, which
 * XLockDisplay does not take; a test hung in XUnlockDisplay). So this speaks
 * the X protocol directly on its own socket: setup, QueryExtension("XFIXES"),
 * XFixes QueryVersion, then HideCursor/ShowCursor on the root window. The
 * hide is per-client and ends automatically if this connection closes. */

typedef unsigned int socklen_t;
typedef long ssize_t;
typedef unsigned long size_t;
struct darwin_sockaddr_un { unsigned char len, family; char path[104]; };
extern int socket(int, int, int);
extern int connect(int, const void *, socklen_t);
extern ssize_t write(int, const void *, size_t);
extern ssize_t read(int, void *, size_t);
extern int close(int);
extern char *getenv(const char *);

static int x_socket = -1;
static unsigned char xfixes_opcode;
static unsigned int root_window;

/* The helpers take the socket: the cursor thread keeps one connection open,
 * visual queries open their own. */
static int write_all_on(int fd, const void *data, size_t length) {
    const unsigned char *bytes = data;
    while (length) {
        ssize_t written = write(fd, bytes, length);
        if (written <= 0)
            return 0;
        bytes += written;
        length -= (size_t)written;
    }
    return 1;
}

static int read_all_on(int fd, void *data, size_t length) {
    unsigned char *bytes = data;
    while (length) {
        ssize_t got = read(fd, bytes, length);
        if (got <= 0)
            return 0;
        bytes += got;
        length -= (size_t)got;
    }
    return 1;
}

/* Read the 32-byte reply to the last request, skipping events. */
static int read_reply_on(int fd, unsigned char reply[32]) {
    for (int guard = 0; guard < 256; guard++) {
        if (!read_all_on(fd, reply, 32))
            return 0;
        if (reply[0] == 0)
            return 0; /* X error */
        if (reply[0] == 1) {
            unsigned int extra = *(unsigned int *)(reply + 4) * 4;
            unsigned char skip[256];
            while (extra) {
                size_t chunk = extra < sizeof skip ? extra : sizeof skip;
                if (!read_all_on(fd, skip, chunk))
                    return 0;
                extra -= (unsigned int)chunk;
            }
            return 1;
        }
    }
    return 0;
}

extern size_t strlen(const char *);
extern void *memcpy(void *, const void *, size_t);
extern int memcmp(const void *, const void *, size_t);
extern int strncmp(const char *, const char *, size_t);
extern int snprintf(char *, unsigned long, const char *, ...);
extern int open(const char *, int, ...);
extern unsigned int getuid(void);

static int read_exact(int fd, void *buf, size_t len) {
    unsigned char *p = (unsigned char *)buf;
    while (len > 0) {
        ssize_t r = read(fd, p, len);
        if (r <= 0) return 0;
        p += r;
        len -= (size_t)r;
    }
    return 1;
}

static int read_u16_be(int fd, unsigned short *out) {
    unsigned char b[2];
    if (!read_exact(fd, b, 2)) return 0;
    *out = (unsigned short)((b[0] << 8) | b[1]);
    return 1;
}

static int find_xauth_cookie(int display_num, char *auth_name, int max_name, unsigned char *auth_cookie, int *cookie_len) {
    const char *candidates[4];
    int count = 0;
    const char *xauth_env = getenv("XAUTHORITY");
    char path_buf[512];
    if (xauth_env && xauth_env[0]) {
        candidates[count++] = xauth_env;
        if (xauth_env[0] == '/' && strncmp(xauth_env, "/Volumes/SystemRoot", 19) != 0) {
            snprintf(path_buf, sizeof(path_buf), "/Volumes/SystemRoot%s", xauth_env);
            candidates[count++] = path_buf;
        }
    }
    const char *home = getenv("HOME");
    char home_buf[512];
    if (home && home[0]) {
        snprintf(home_buf, sizeof(home_buf), "%s/.Xauthority", home);
        candidates[count++] = home_buf;
    }
    char sys_home_buf[512];
    if (home && home[0] && strncmp(home, "/Volumes/SystemRoot", 19) != 0) {
        snprintf(sys_home_buf, sizeof(sys_home_buf), "/Volumes/SystemRoot%s/.Xauthority", home);
        candidates[count++] = sys_home_buf;
    }

    char disp_str[16];
    snprintf(disp_str, sizeof(disp_str), "%d", display_num);
    size_t disp_str_len = strlen(disp_str);

    for (int c = 0; c < count; c++) {
        int fd = open(candidates[c], 0 /* O_RDONLY */);
        if (fd < 0) continue;

        unsigned short family, addr_len, num_len, name_l, data_l;
        unsigned char skip[256];
        char num[64];

        while (read_u16_be(fd, &family)) {
            if (!read_u16_be(fd, &addr_len)) break;
            while (addr_len > 0) {
                size_t ch = addr_len < sizeof(skip) ? addr_len : sizeof(skip);
                if (!read_exact(fd, skip, ch)) goto next_candidate;
                addr_len -= ch;
            }
            if (!read_u16_be(fd, &num_len)) break;
            if (num_len < sizeof(num)) {
                if (!read_exact(fd, num, num_len)) break;
                num[num_len] = 0;
            } else {
                goto next_candidate;
            }
            if (!read_u16_be(fd, &name_l)) break;
            char nbuf[64];
            if (name_l < sizeof(nbuf)) {
                if (!read_exact(fd, nbuf, name_l)) break;
                nbuf[name_l] = 0;
            } else {
                goto next_candidate;
            }
            if (!read_u16_be(fd, &data_l)) break;
            unsigned char dbuf[64];
            if (data_l < sizeof(dbuf)) {
                if (!read_exact(fd, dbuf, data_l)) break;
            } else {
                goto next_candidate;
            }

            if (num_len == 0 || (num_len == disp_str_len && memcmp(num, disp_str, disp_str_len) == 0)) {
                if (name_l < max_name && data_l <= 64) {
                    memcpy(auth_name, nbuf, name_l);
                    auth_name[name_l] = 0;
                    memcpy(auth_cookie, dbuf, data_l);
                    *cookie_len = (int)data_l;
                    close(fd);
                    return 1;
                }
            }
        }
next_candidate:
        close(fd);
    }
    return 0;
}

static int display_number(void) {
    const char *display = getenv("DISPLAY");
    int number = 0;
    if (!display)
        return 0;
    while (*display && *display != ':')
        display++;
    if (*display == ':')
        display++;
    while (*display >= '0' && *display <= '9')
        number = number * 10 + (*display++ - '0');
    return number;
}

static int try_connect_socket(const char *path, int *out) {
    struct darwin_sockaddr_un address = {0};
    int length = 0;
    while (path[length] && length < (int)sizeof(address.path) - 1) {
        address.path[length] = path[length];
        length++;
    }
    address.path[length] = 0;
    address.family = 1; /* AF_UNIX */
    address.len = (unsigned char)(2 + length + 1);

    int fd = socket(1, 1 /* SOCK_STREAM */, 0);
    if (fd < 0) return 0;
    if (connect(fd, &address, sizeof(address)) != 0) {
        close(fd);
        return 0;
    }
    *out = fd;
    return 1;
}

static int connect_display_on(int *out) {
    int number = display_number();
    char path[256];

    // 1. Host X11 socket under Darling's SystemRoot
    snprintf(path, sizeof(path), "/Volumes/SystemRoot/tmp/.X11-unix/X%d", number);
    if (try_connect_socket(path, out)) return 1;

    // 2. Direct /tmp socket
    snprintf(path, sizeof(path), "/tmp/.X11-unix/X%d", number);
    if (try_connect_socket(path, out)) return 1;

    // 3. User runtime directory X11 display socket
    unsigned int uid = getuid();
    snprintf(path, sizeof(path), "/Volumes/SystemRoot/run/user/%u/X11-display", uid);
    if (try_connect_socket(path, out)) return 1;

    return 0;
}

/* Connection setup; returns the first screen's root window and visual. */
static int setup_on(int fd, unsigned int *root, unsigned int *visual) {
    char auth_name[64] = {0};
    unsigned char auth_cookie[64] = {0};
    int cookie_len = 0;
    int has_auth = find_xauth_cookie(display_number(), auth_name, sizeof(auth_name), auth_cookie, &cookie_len);

    unsigned short name_len = has_auth ? (unsigned short)strlen(auth_name) : 0;
    unsigned short data_len = has_auth ? (unsigned short)cookie_len : 0;

    unsigned char request[12] = {
        'l', 0,
        11, 0,
        0, 0,
        (unsigned char)(name_len & 0xff), (unsigned char)((name_len >> 8) & 0xff),
        (unsigned char)(data_len & 0xff), (unsigned char)((data_len >> 8) & 0xff),
        0, 0
    };

    if (!write_all_on(fd, request, sizeof(request)))
        return 0;

    if (has_auth) {
        unsigned int name_pad = (4 - (name_len % 4)) % 4;
        unsigned char pad[4] = {0, 0, 0, 0};
        if (!write_all_on(fd, auth_name, name_len) ||
            (name_pad > 0 && !write_all_on(fd, pad, name_pad)))
            return 0;

        unsigned int data_pad = (4 - (data_len % 4)) % 4;
        if (!write_all_on(fd, auth_cookie, (size_t)data_len) ||
            (data_pad > 0 && !write_all_on(fd, pad, data_pad)))
            return 0;
    }

    unsigned char header[8];
    if (!read_all_on(fd, header, sizeof header) || header[0] != 1)
        return 0;
    unsigned int body_length = *(unsigned short *)(header + 6) * 4u;
    static unsigned char body[1 << 16];
    if (body_length > sizeof body || !read_all_on(fd, body, body_length))
        return 0;
    unsigned int vendor_length = *(unsigned short *)(body + 16);
    unsigned int formats = body[21];
    unsigned int screen = 32 + ((vendor_length + 3) & ~3u) + 8 * formats;
    if (screen + 36 > body_length)
        return 0;
    *root = *(unsigned int *)(body + screen);
    if (visual)
        *visual = *(unsigned int *)(body + screen + 32);
    return 1;
}

static int query_xfixes(void) {
    unsigned char request[16] = {98 /* QueryExtension */, 0, 4, 0, 6, 0, 0, 0,
                                 'X', 'F', 'I', 'X', 'E', 'S', 0, 0};
    unsigned char reply[32];
    if (!write_all_on(x_socket, request, sizeof request) || !read_reply_on(x_socket, reply) || !reply[8])
        return 0;
    xfixes_opcode = reply[9];
    unsigned char version[12] = {xfixes_opcode, 0 /* QueryVersion */, 3, 0, 5, 0, 0, 0, 0, 0, 0, 0};
    return write_all_on(x_socket, version, sizeof version) && read_reply_on(x_socket, reply);
}

int macoblox_raw_xfixes_open(void) {
    if (x_socket >= 0)
        return 1;
    if (!connect_display_on(&x_socket))
        return 0;
    if (!setup_on(x_socket, &root_window, 0) || !query_xfixes()) {
        close(x_socket);
        x_socket = -1;
        return 0;
    }
    return 1;
}

int macoblox_raw_xfixes_set_hidden(int hidden) {
    if (x_socket < 0)
        return 0;
    unsigned char request[8] = {xfixes_opcode, hidden ? 29 /* HideCursor */ : 30 /* ShowCursor */,
                                2, 0};
    *(unsigned int *)(request + 4) = root_window;
    return write_all_on(x_socket, request, sizeof request);
}

/* The screen's default visual and, if `window` is not 0, that window's
 * visual (GetWindowAttributes). Opens and closes its own connection.
 * Returns 0 when the X server cannot be reached. */
int macoblox_raw_x_visuals(unsigned int window, unsigned int *root_visual, unsigned int *window_visual) {
    int fd;
    unsigned int root;
    if (!connect_display_on(&fd))
        return 0;
    int ok = setup_on(fd, &root, root_visual);
    if (ok && window && window_visual) {
        unsigned char request[8] = {3 /* GetWindowAttributes */, 0, 2, 0};
        unsigned char reply[32];
        *(unsigned int *)(request + 4) = window;
        ok = write_all_on(fd, request, sizeof request) && read_reply_on(fd, reply);
        if (ok)
            *window_visual = *(unsigned int *)(reply + 8);
    }
    close(fd);
    return ok;
}
