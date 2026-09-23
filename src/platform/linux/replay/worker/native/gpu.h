#ifndef BOLTSNAP_CURSOR_GPU_H
#define BOLTSNAP_CURSOR_GPU_H
#include <stdint.h>
/* Thread-confined. All imports borrow FDs until the function returns. */
typedef struct BsGpu BsGpu;
BsGpu *bs_gpu_open(const char *node, int width, int height, char *error,
                   int error_size);
void bs_gpu_close(BsGpu *gpu);
int bs_gpu_background(BsGpu *gpu, int width, int height, uint32_t fourcc,
                      uint64_t modifier, int planes, const int *fds,
                      const uint32_t *strides, const uint32_t *offsets);
int bs_gpu_rgba_background(BsGpu *gpu, const uint8_t *rgba);
int bs_gpu_cursor(BsGpu *gpu, int width, int height, const uint8_t *rgba);
/* Pixel coordinates are top-left relative. Negative cursor positions clip. */
int bs_gpu_draw(BsGpu *gpu, int visible, float x, float y);
int bs_gpu_readback(BsGpu *gpu, uint8_t *rgba, int bytes);
/* Video packets are NUT-framed; output FD remains caller-owned. Encoding uses
 * the exact requested codec. Only explicit libx264 uses a CPU frame transfer.
 */
int bs_gpu_encoder(BsGpu *gpu, const char *codec, int fps, int fd);
int bs_gpu_encode(BsGpu *gpu, int64_t pts);
int bs_gpu_finish(BsGpu *gpu);
const char *bs_gpu_error(BsGpu *gpu);
#endif
