#define _GNU_SOURCE
#include <endian.h>
#include <errno.h>
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <unistd.h>
#include <wayland-client.h>
#include "toplevel-export.h"

static uint32_t target_width = 720, target_height = 450;

/* One short-lived process per thumbnail captures exactly one frame, sent as
 * little-endian width/height followed by tightly packed, premultiplied RGBA. */
struct capture {
    struct wl_display *display;
    struct wl_shm *shm;
    struct hyprland_toplevel_export_manager_v1 *manager;
    struct wl_buffer *buffer;
    void *pixels;
    size_t size;
    uint32_t width, height, stride, format, flags;
    int done, failed;
};

static void global(void *data, struct wl_registry *registry, uint32_t name,
                   const char *interface, uint32_t version) {
    (void)version;
    struct capture *c = data;
    if (!strcmp(interface, wl_shm_interface.name))
        c->shm = wl_registry_bind(registry, name, &wl_shm_interface, 1);
    if (!strcmp(interface, hyprland_toplevel_export_manager_v1_interface.name))
        c->manager = wl_registry_bind(registry, name,
            &hyprland_toplevel_export_manager_v1_interface, 1);
}
static void removed(void *data, struct wl_registry *registry, uint32_t name) {
    (void)data; (void)registry; (void)name;
}
static const struct wl_registry_listener registry_listener = {global, removed};

static int supported(uint32_t f) {
    return f == WL_SHM_FORMAT_ARGB8888 || f == WL_SHM_FORMAT_XRGB8888 ||
           f == WL_SHM_FORMAT_ABGR8888 || f == WL_SHM_FORMAT_XBGR8888;
}

static void buffer_info(void *data, struct hyprland_toplevel_export_frame_v1 *f,
                        uint32_t format, uint32_t width, uint32_t height,
                        uint32_t stride) {
    (void)f;
    struct capture *c = data;
    c->format = format; c->width = width; c->height = height; c->stride = stride;
}
static void damage(void *data, struct hyprland_toplevel_export_frame_v1 *f,
                   uint32_t x, uint32_t y, uint32_t w, uint32_t h) {
    (void)data; (void)f; (void)x; (void)y; (void)w; (void)h;
}
static void flags(void *data, struct hyprland_toplevel_export_frame_v1 *f, uint32_t v) {
    (void)f; ((struct capture *)data)->flags = v;
}
static void ready(void *data, struct hyprland_toplevel_export_frame_v1 *f,
                  uint32_t hi, uint32_t lo, uint32_t ns) {
    (void)f; (void)hi; (void)lo; (void)ns;
    ((struct capture *)data)->done = 1;
}
static void failed(void *data, struct hyprland_toplevel_export_frame_v1 *f) {
    (void)f; ((struct capture *)data)->failed = 1;
}
static void dmabuf(void *data, struct hyprland_toplevel_export_frame_v1 *f,
                   uint32_t format, uint32_t w, uint32_t h) {
    (void)data; (void)f; (void)format; (void)w; (void)h;
}

static void release_buffer(struct capture *c) {
    if (c->buffer) wl_buffer_destroy(c->buffer);
    if (c->pixels) munmap(c->pixels, c->size);
    c->buffer = NULL; c->pixels = NULL;
}

static void buffer_done(void *data, struct hyprland_toplevel_export_frame_v1 *f) {
    struct capture *c = data;
    if (!supported(c->format) || !c->width || !c->height ||
        c->width > 16384 || c->height > 16384 ||
        c->stride < c->width * 4 || c->stride > 16384 * 4 ||
        (uint64_t)c->stride * c->height > 256 * 1024 * 1024) {
        c->failed = 1; return;
    }
    c->size = (size_t)c->stride * c->height;
    int fd = memfd_create("hyprshell-preview", MFD_CLOEXEC);
    if (fd < 0) { c->failed = 1; return; }
    if (ftruncate(fd, (off_t)c->size) < 0) {
        close(fd); c->failed = 1; return;
    }
    c->pixels = mmap(NULL, c->size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
    if (c->pixels == MAP_FAILED) {
        c->pixels = NULL; close(fd); c->failed = 1; return;
    }
    struct wl_shm_pool *pool = wl_shm_create_pool(c->shm, fd, (int)c->size);
    close(fd);
    c->buffer = wl_shm_pool_create_buffer(pool, 0, (int)c->width, (int)c->height,
                                         (int)c->stride, c->format);
    wl_shm_pool_destroy(pool);
    hyprland_toplevel_export_frame_v1_copy(f, c->buffer, 1);
}
static const struct hyprland_toplevel_export_frame_v1_listener frame_listener = {
    .buffer = buffer_info, .damage = damage, .flags = flags, .ready = ready,
    .failed = failed, .linux_dmabuf = dmabuf, .buffer_done = buffer_done,
};

/* Bound waits so closed/hidden windows and compositor failures do not leave
 * stuck capture processes. The GTK side also kills this process on unmap. */
static int dispatch(struct wl_display *display) {
    while (wl_display_prepare_read(display) != 0)
        if (wl_display_dispatch_pending(display) < 0) return -1;
    if (wl_display_flush(display) < 0 && errno != EAGAIN) {
        wl_display_cancel_read(display); return -1;
    }
    struct pollfd p = {.fd = wl_display_get_fd(display), .events = POLLIN};
    int result;
    do { result = poll(&p, 1, 2000); } while (result < 0 && errno == EINTR);
    if (result <= 0 || !(p.revents & POLLIN)) {
        wl_display_cancel_read(display); return -1;
    }
    if (wl_display_read_events(display) < 0) return -1;
    return wl_display_dispatch_pending(display);
}

static void rgba(const struct capture *c, uint32_t x, uint32_t y, uint8_t out[4]) {
    if (c->flags & HYPRLAND_TOPLEVEL_EXPORT_FRAME_V1_FLAGS_Y_INVERT)
        y = c->height - y - 1;
    const uint8_t *p = (const uint8_t *)c->pixels + y * c->stride + x * 4;
    int bgr = c->format == WL_SHM_FORMAT_ARGB8888 || c->format == WL_SHM_FORMAT_XRGB8888;
    int alpha = c->format == WL_SHM_FORMAT_ARGB8888 || c->format == WL_SHM_FORMAT_ABGR8888;
    uint8_t a = alpha ? p[3] : 255;
    /* Preserve premultiplication for GDK, avoiding three divisions per pixel. */
    for (int i = 0; i < 3; ++i)
        out[i] = a ? p[bgr ? 2 - i : i] : 0;
    out[3] = a;
}

static int emit(const struct capture *c) {
    /* Two physical pixels per logical pixel keep the larger cards crisp. */
    double scale = (double)target_width / c->width;
    if ((double)target_height / c->height < scale) scale = (double)target_height / c->height;
    if (scale > 1) scale = 1;
    uint32_t w = (uint32_t)(c->width * scale), h = (uint32_t)(c->height * scale);
    if (!w) w = 1;
    if (!h) h = 1;
    uint8_t pixels[720 * 450 * 4];
    for (uint32_t y = 0; y < h; ++y)
        for (uint32_t x = 0; x < w; ++x)
            rgba(c, x * c->width / w, y * c->height / h, pixels + (y * w + x) * 4);
    uint32_t header[] = {htole32(w), htole32(h)};
    return fwrite(header, sizeof(header), 1, stdout) == 1 &&
           fwrite(pixels, (size_t)w * h * 4, 1, stdout) == 1 && fflush(stdout) == 0;
}

int main(int argc, char **argv) {
    if (argc != 2 && argc != 4) {
        fprintf(stderr, "Usage: %s WINDOW_ADDRESS_HEX [MAX_WIDTH MAX_HEIGHT]\n", argv[0]);
        return 2;
    }
    char *end;
    errno = 0;
    unsigned long long address = strtoull(argv[1], &end, 16);
    if (errno || *end || end == argv[1] || !address) return 2;
    if (argc == 4) {
        errno = 0;
        unsigned long width = strtoul(argv[2], &end, 10);
        if (errno || *end || end == argv[2] || !width || width > 720) return 2;
        unsigned long height = strtoul(argv[3], &end, 10);
        if (errno || *end || end == argv[3] || !height || height > 450) return 2;
        target_width = (uint32_t)width; target_height = (uint32_t)height;
    }
    struct capture c = {0};
    c.display = wl_display_connect(NULL);
    if (!c.display) { fprintf(stderr, "Preview: cannot connect to Wayland\n"); return 1; }
    struct wl_registry *registry = wl_display_get_registry(c.display);
    wl_registry_add_listener(registry, &registry_listener, &c);
    /* Registry sync is asynchronous too, to retain the bounded wait. */
    for (int i = 0; i < 8 && (!c.manager || !c.shm); ++i)
        if (dispatch(c.display) < 0) break;
    if (!c.manager || !c.shm) {
        fprintf(stderr, "Preview: Hyprland toplevel export is unavailable\n");
        wl_display_disconnect(c.display); return 1;
    }
    struct hyprland_toplevel_export_frame_v1 *frame =
        hyprland_toplevel_export_manager_v1_capture_toplevel(c.manager, 0, (uint32_t)address);
    hyprland_toplevel_export_frame_v1_add_listener(frame, &frame_listener, &c);
    while (!c.done && !c.failed)
        if (dispatch(c.display) < 0) { c.failed = 1; break; }
    int sent = !c.failed && emit(&c);
    hyprland_toplevel_export_frame_v1_destroy(frame);
    release_buffer(&c);
    hyprland_toplevel_export_manager_v1_destroy(c.manager);
    wl_shm_destroy(c.shm);
    wl_registry_destroy(registry);
    wl_display_disconnect(c.display);
    return sent ? 0 : 1;
}
