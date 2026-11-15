/* Test padding, pixel formats, alpha and inverted captures without a display. */
#define main capture_program_main
#include "capture.c"
#undef main
#include <assert.h>

int main(void) {
    uint8_t bytes[] = {30, 20, 10, 255, 99, 99, 99, 99,
                       60, 40, 20, 128, 88, 88, 88, 88};
    struct capture c = {.pixels = bytes, .width = 1, .height = 2,
                        .stride = 8, .format = WL_SHM_FORMAT_ARGB8888};
    uint8_t pixel[4];
    rgba(&c, 0, 0, pixel);
    assert(!memcmp(pixel, (uint8_t[]){10, 20, 30, 255}, 4));
    c.flags = HYPRLAND_TOPLEVEL_EXPORT_FRAME_V1_FLAGS_Y_INVERT;
    rgba(&c, 0, 0, pixel);
    assert(!memcmp(pixel, (uint8_t[]){20, 40, 60, 128}, 4));
    c.flags = 0;
    c.format = WL_SHM_FORMAT_XBGR8888;
    bytes[3] = 0;
    rgba(&c, 0, 0, pixel);
    assert(!memcmp(pixel, (uint8_t[]){30, 20, 10, 255}, 4));
    c.format = WL_SHM_FORMAT_ABGR8888;
    rgba(&c, 0, 0, pixel);
    assert(!memcmp(pixel, (uint8_t[]){0, 0, 0, 0}, 4));
    assert(!supported(WL_SHM_FORMAT_RGB565));
    return 0;
}
