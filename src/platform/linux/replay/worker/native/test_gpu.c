/* Headless, synthetic pixels only. Requires an explicit render-node argument.
 */
#include "gpu.h"
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GLES3/gl3.h>
#include <GLES2/gl2ext.h>
#include <gbm.h>
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#define W 320
#define H 180
#define CHECK(x)                                                               \
    do {                                                                       \
        if (!(x)) {                                                            \
            fprintf(stderr, "line %d: %s (%s)\n", __LINE__, #x,                \
                    g ? bs_gpu_error(g) : error);                              \
            return 1;                                                          \
        }                                                                      \
    } while (0)
int main(int argc, char **argv) {
    char error[512] = {0};
    BsGpu *g = NULL;
    CHECK(argc == 2 || argc == 4);
    int fd = -1;
    g = bs_gpu_open(argv[1], W, H, error, sizeof error);
    CHECK(g);
    if (argc == 4) {
        fd = open(argv[3], O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC, 0600);
        CHECK(fd >= 0);
        CHECK(bs_gpu_encoder(g, argv[2], 60, fd) == 0);
    }
    unsigned char background[W * H * 4], readback[W * H * 4], cursor[8 * 8 * 4];
    for (int y = 0; y < H; y++)
        for (int x = 0; x < W; x++) {
            unsigned char *p = background + 4 * (y * W + x);
            p[0] = 16 + (x % 2) * 16;
            p[1] = 32 + (y % 2) * 32;
            p[2] = 48;
            p[3] = 255;
        }
    for (int i = 0; i < 64; i++) {
        cursor[4 * i] = 64;
        cursor[4 * i + 1] = 0;
        cursor[4 * i + 2] = 0;
        cursor[4 * i + 3] = 128;
    }
    /* Populate a real exported GBM allocation, import it as the background,
     * then release the producer's handles. EGL must retain the storage. */
    int render = open(argv[1], O_RDWR | O_CLOEXEC);
    CHECK(render >= 0);
    struct gbm_device *device = gbm_create_device(render);
    CHECK(device);
    struct gbm_bo *bo =
        gbm_bo_create(device, W, H, GBM_FORMAT_ABGR8888, GBM_BO_USE_RENDERING);
    CHECK(bo);
    int dma = gbm_bo_get_fd(bo);
    CHECK(dma >= 0);
    uint64_t modifier = gbm_bo_get_modifier(bo);
    uint32_t stride = gbm_bo_get_stride(bo), offset = 0;
    EGLint attributes[] = {EGL_WIDTH,
                           W,
                           EGL_HEIGHT,
                           H,
                           EGL_LINUX_DRM_FOURCC_EXT,
                           GBM_FORMAT_ABGR8888,
                           EGL_DMA_BUF_PLANE0_FD_EXT,
                           dma,
                           EGL_DMA_BUF_PLANE0_OFFSET_EXT,
                           0,
                           EGL_DMA_BUF_PLANE0_PITCH_EXT,
                           (EGLint)stride,
                           EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT,
                           (EGLint)modifier,
                           EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT,
                           (EGLint)(modifier >> 32),
                           EGL_NONE};
    PFNEGLCREATEIMAGEKHRPROC create =
        (void *)eglGetProcAddress("eglCreateImageKHR");
    PFNEGLDESTROYIMAGEKHRPROC destroy =
        (void *)eglGetProcAddress("eglDestroyImageKHR");
    PFNGLEGLIMAGETARGETTEXTURE2DOESPROC target =
        (void *)eglGetProcAddress("glEGLImageTargetTexture2DOES");
    CHECK(create && destroy && target);
    EGLDisplay display = eglGetCurrentDisplay();
    EGLImageKHR image = create(display, EGL_NO_CONTEXT, EGL_LINUX_DMA_BUF_EXT,
                               NULL, attributes);
    CHECK(image != EGL_NO_IMAGE_KHR);
    GLuint texture;
    glGenTextures(1, &texture);
    glBindTexture(GL_TEXTURE_2D, texture);
    target(GL_TEXTURE_2D, image);
    glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, W, H, GL_RGBA, GL_UNSIGNED_BYTE,
                    background);
    glFinish();
    CHECK(glGetError() == GL_NO_ERROR);
    CHECK(bs_gpu_background(g, W, H, GBM_FORMAT_ABGR8888, modifier, 1, &dma,
                            &stride, &offset) == 0);
    glDeleteTextures(1, &texture);
    destroy(display, image);
    close(dma);
    gbm_bo_destroy(bo);
    gbm_device_destroy(device);
    close(render);
    CHECK(bs_gpu_cursor(g, 8, 8, cursor) == 0);
    for (int frame = 0; frame < 80; frame++) {
        int cx = frame * 5 - 8, cy = frame % 2 ? H - 4 : -4,
            visible = frame % 3;
        CHECK(bs_gpu_draw(g, visible, cx, cy) == 0);
        CHECK(bs_gpu_readback(g, readback, sizeof readback) == 0);
        if (fd >= 0)
            CHECK(bs_gpu_encode(g, frame) == 0);
        for (int y = 0; y < H; y++)
            for (int x = 0; x < W; x++)
                for (int c = 0; c < 4; c++) {
                    int at = 4 * (y * W + x) + c, expected = background[at];
                    if (visible && x >= cx && x < cx + 8 && y >= cy &&
                        y < cy + 8)
                        expected = (c == 0   ? 64
                                    : c == 3 ? 128
                                             : 0) +
                                   (background[at] * 127 + 127) / 255;
                    if (abs(readback[at] - expected) > 1) {
                        fprintf(stderr,
                                "frame %d pixel %d,%d channel %d: got %d "
                                "expected %d\n",
                                frame, x, y, c, readback[at], expected);
                        return 1;
                    }
                }
    }
    if (fd >= 0) {
        CHECK(bs_gpu_finish(g) == 0);
        CHECK(close(fd) == 0);
    }
    bs_gpu_close(g);
    puts("80 moving/hidden/clipped premultiplied-alpha cursor frames: pixels "
         "OK");
    return 0;
}
