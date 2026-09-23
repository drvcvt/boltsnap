/* Optional Linux compositor. Public FFmpeg/Vulkan/EGL APIs only.
 * The context and all calls belong to one thread. GPU fences bound ownership:
 * an imported background is no longer accessed when draw returns. */
#include "gpu.h"
#include <EGL/egl.h>
#include <EGL/eglext.h>
#include <GLES3/gl3.h>
#include <GLES2/gl2ext.h>
#include <errno.h>
#include <fcntl.h>
#include <gbm.h>
#include <libavcodec/avcodec.h>
#include <libavfilter/avfilter.h>
#include <libavfilter/buffersink.h>
#include <libavfilter/buffersrc.h>
#include <libavformat/avformat.h>
#include <libavutil/error.h>
#include <libavutil/hwcontext.h>
#include <libavutil/hwcontext_vulkan.h>
#include <libavutil/opt.h>
#include <libavutil/pixdesc.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#define GLFN(type, name) type name
struct BsGpu {
    int node, width, height, cursor_width, cursor_height;
    struct gbm_device *gbm;
    EGLDisplay display;
    EGLContext context;
    EGLImageKHR background_image;
    GLuint background, cursor, output, memory, framebuffer, program, gl_sem;
    AVBufferRef *device, *frames;
    AVFrame *frame;
    AVHWDeviceContext *dc;
    AVVulkanDeviceContext *vd;
    VkCommandPool pool;
    VkCommandBuffer command;
    VkSemaphore semaphore;
    VkFence fence;
    VkQueue queue;
    uint32_t family;
    AVFilterGraph *graph;
    AVFilterContext *source, *sink;
    AVCodecContext *encoder;
    AVFormatContext *mux;
    AVIOContext *io;
    AVPacket *packet;
    int output_fd, header_written, finished;
    char error[512];
    GLFN(PFNEGLCREATEIMAGEKHRPROC, create_image);
    GLFN(PFNEGLDESTROYIMAGEKHRPROC, destroy_image);
    GLFN(PFNGLEGLIMAGETARGETTEXTURE2DOESPROC, target_image);
    GLFN(PFNGLCREATEMEMORYOBJECTSEXTPROC, create_memory);
    GLFN(PFNGLDELETEMEMORYOBJECTSEXTPROC, delete_memory);
    GLFN(PFNGLMEMORYOBJECTPARAMETERIVEXTPROC, memory_parameter);
    GLFN(PFNGLIMPORTMEMORYFDEXTPROC, import_memory);
    GLFN(PFNGLTEXSTORAGEMEM2DEXTPROC, texture_storage);
    GLFN(PFNGLGENSEMAPHORESEXTPROC, create_semaphores);
    GLFN(PFNGLDELETESEMAPHORESEXTPROC, delete_semaphores);
    GLFN(PFNGLIMPORTSEMAPHOREFDEXTPROC, import_semaphore);
    GLFN(PFNGLWAITSEMAPHOREEXTPROC, wait_semaphore);
    GLFN(PFNGLSIGNALSEMAPHOREEXTPROC, signal_semaphore);
};
#define FAIL(g, ...)                                                           \
    do {                                                                       \
        snprintf((g)->error, sizeof((g)->error), __VA_ARGS__);                 \
        return -1;                                                             \
    } while (0)
#define REQUIRE(g, x)                                                          \
    do {                                                                       \
        if (!(x))                                                              \
            FAIL(g, "%s failed (EGL 0x%x, GL 0x%x)", #x, eglGetError(),        \
                 glGetError());                                                \
    } while (0)
#define VK(g, x)                                                               \
    do {                                                                       \
        VkResult r = (x);                                                      \
        if (r != VK_SUCCESS)                                                   \
            FAIL(g, "%s: Vulkan %d", #x, r);                                   \
    } while (0)
#define AV(g, x)                                                               \
    do {                                                                       \
        int r = (x);                                                           \
        if (r < 0) {                                                           \
            char e[128];                                                       \
            av_strerror(r, e, sizeof e);                                       \
            FAIL(g, "%s: %s", #x, e);                                          \
        }                                                                      \
    } while (0)
#define LOAD(g, field, type, name)                                             \
    do {                                                                       \
        (g)->field = (type)eglGetProcAddress(name);                            \
        REQUIRE(g, (g)->field);                                                \
    } while (0)

static int shader(BsGpu *g, GLenum kind, const char *text, GLuint *out) {
    *out = glCreateShader(kind);
    REQUIRE(g, *out);
    glShaderSource(*out, 1, &text, NULL);
    glCompileShader(*out);
    GLint ok;
    glGetShaderiv(*out, GL_COMPILE_STATUS, &ok);
    if (!ok) {
        glGetShaderInfoLog(*out, sizeof g->error, NULL, g->error);
        return -1;
    }
    return 0;
}
static void texture_parameters(void) {
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
}
static int initialize(BsGpu *g, const char *node) {
    g->node = open(node, O_RDWR | O_CLOEXEC);
    REQUIRE(g, g->node >= 0);
    g->gbm = gbm_create_device(g->node);
    REQUIRE(g, g->gbm);
    g->display = eglGetPlatformDisplay(EGL_PLATFORM_GBM_KHR, g->gbm, NULL);
    REQUIRE(g, g->display != EGL_NO_DISPLAY);
    REQUIRE(g, eglInitialize(g->display, NULL, NULL));
    REQUIRE(g, eglBindAPI(EGL_OPENGL_ES_API));
    EGLint attrs[] = {EGL_CONTEXT_CLIENT_VERSION, 3, EGL_NONE};
    g->context =
        eglCreateContext(g->display, EGL_NO_CONFIG_KHR, EGL_NO_CONTEXT, attrs);
    REQUIRE(g, g->context != EGL_NO_CONTEXT);
    REQUIRE(g, eglMakeCurrent(g->display, EGL_NO_SURFACE, EGL_NO_SURFACE,
                              g->context));
    const char *extensions = (const char *)glGetString(GL_EXTENSIONS);
    REQUIRE(g, extensions && strstr(extensions, "GL_EXT_memory_object_fd") &&
                   strstr(extensions, "GL_EXT_semaphore_fd"));
    LOAD(g, create_image, PFNEGLCREATEIMAGEKHRPROC, "eglCreateImageKHR");
    LOAD(g, destroy_image, PFNEGLDESTROYIMAGEKHRPROC, "eglDestroyImageKHR");
    LOAD(g, target_image, PFNGLEGLIMAGETARGETTEXTURE2DOESPROC,
         "glEGLImageTargetTexture2DOES");
    LOAD(g, create_memory, PFNGLCREATEMEMORYOBJECTSEXTPROC,
         "glCreateMemoryObjectsEXT");
    LOAD(g, delete_memory, PFNGLDELETEMEMORYOBJECTSEXTPROC,
         "glDeleteMemoryObjectsEXT");
    LOAD(g, memory_parameter, PFNGLMEMORYOBJECTPARAMETERIVEXTPROC,
         "glMemoryObjectParameterivEXT");
    LOAD(g, import_memory, PFNGLIMPORTMEMORYFDEXTPROC, "glImportMemoryFdEXT");
    LOAD(g, texture_storage, PFNGLTEXSTORAGEMEM2DEXTPROC,
         "glTexStorageMem2DEXT");
    LOAD(g, create_semaphores, PFNGLGENSEMAPHORESEXTPROC, "glGenSemaphoresEXT");
    LOAD(g, delete_semaphores, PFNGLDELETESEMAPHORESEXTPROC,
         "glDeleteSemaphoresEXT");
    LOAD(g, import_semaphore, PFNGLIMPORTSEMAPHOREFDEXTPROC,
         "glImportSemaphoreFdEXT");
    LOAD(g, wait_semaphore, PFNGLWAITSEMAPHOREEXTPROC, "glWaitSemaphoreEXT");
    LOAD(g, signal_semaphore, PFNGLSIGNALSEMAPHOREEXTPROC,
         "glSignalSemaphoreEXT");
    AV(g, av_hwdevice_ctx_create(&g->device, AV_HWDEVICE_TYPE_VULKAN, "0", NULL,
                                 0));
    g->dc = (void *)g->device->data;
    g->vd = g->dc->hwctx;
    // Reject cross-device imports before exporting memory. Index 0 is an
    // explicit initial limitation, not an assumption that the render node
    // matches it.
    PFNGLGETUNSIGNEDBYTEI_VEXTPROC get_uuid =
        (void *)eglGetProcAddress("glGetUnsignedBytei_vEXT");
    REQUIRE(g, get_uuid);
    VkPhysicalDeviceIDProperties ids = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_ID_PROPERTIES};
    VkPhysicalDeviceProperties2 props = {
        .sType = VK_STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2, .pNext = &ids};
    vkGetPhysicalDeviceProperties2(g->vd->phys_dev, &props);
    GLubyte uuid[GL_UUID_SIZE_EXT];
    get_uuid(GL_DEVICE_UUID_EXT, 0, uuid);
    REQUIRE(g, glGetError() == GL_NO_ERROR);
    REQUIRE(g, memcmp(uuid, ids.deviceUUID, sizeof uuid) == 0);
    g->frames = av_hwframe_ctx_alloc(g->device);
    REQUIRE(g, g->frames);
    AVHWFramesContext *fc = (void *)g->frames->data;
    fc->format = AV_PIX_FMT_VULKAN;
    fc->sw_format = AV_PIX_FMT_RGBA;
    fc->width = g->width;
    fc->height = g->height;
    ((AVVulkanFramesContext *)fc->hwctx)->usage =
        VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT;
    AV(g, av_hwframe_ctx_init(g->frames));
    g->frame = av_frame_alloc();
    REQUIRE(g, g->frame);
    AV(g, av_hwframe_get_buffer(g->frames, g->frame, 0));
    AVVkFrame *vf = (void *)g->frame->data[0];
    PFN_vkGetMemoryFdKHR get_fd =
        (void *)vkGetDeviceProcAddr(g->vd->act_dev, "vkGetMemoryFdKHR");
    REQUIRE(g, get_fd);
    VkMemoryGetFdInfoKHR info = {
        .sType = VK_STRUCTURE_TYPE_MEMORY_GET_FD_INFO_KHR,
        .memory = vf->mem[0],
        .handleType = VK_EXTERNAL_MEMORY_HANDLE_TYPE_OPAQUE_FD_BIT};
    int fd = -1;
    VK(g, get_fd(g->vd->act_dev, &info, &fd));
    g->create_memory(1, &g->memory);
    GLint dedicated = GL_TRUE;
    g->memory_parameter(g->memory, GL_DEDICATED_MEMORY_OBJECT_EXT, &dedicated);
    // EXT_memory_object_fd transfers FD ownership to GL on import.
    g->import_memory(g->memory, vf->size[0], GL_HANDLE_TYPE_OPAQUE_FD_EXT, fd);
    REQUIRE(g, glGetError() == GL_NO_ERROR);
    glGenTextures(1, &g->output);
    glBindTexture(GL_TEXTURE_2D, g->output);
    g->texture_storage(GL_TEXTURE_2D, 1, GL_RGBA8, g->width, g->height,
                       g->memory, vf->offset[0]);
    REQUIRE(g, glGetError() == GL_NO_ERROR);
    glGenFramebuffers(1, &g->framebuffer);
    glBindFramebuffer(GL_FRAMEBUFFER, g->framebuffer);
    glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D,
                           g->output, 0);
    REQUIRE(g, glCheckFramebufferStatus(GL_FRAMEBUFFER) ==
                   GL_FRAMEBUFFER_COMPLETE);
    int found = 0;
    for (int i = 0; i < g->vd->nb_qf; i++)
        if (g->vd->qf[i].flags & VK_QUEUE_GRAPHICS_BIT) {
            g->family = g->vd->qf[i].idx;
            found = 1;
            break;
        }
    REQUIRE(g, found);
    vkGetDeviceQueue(g->vd->act_dev, g->family, 0, &g->queue);
    VkCommandPoolCreateInfo pi = {
        .sType = VK_STRUCTURE_TYPE_COMMAND_POOL_CREATE_INFO,
        .flags = VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT,
        .queueFamilyIndex = g->family};
    VK(g, vkCreateCommandPool(g->vd->act_dev, &pi, NULL, &g->pool));
    VkCommandBufferAllocateInfo ai = {
        .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_ALLOCATE_INFO,
        .commandPool = g->pool,
        .level = VK_COMMAND_BUFFER_LEVEL_PRIMARY,
        .commandBufferCount = 1};
    VK(g, vkAllocateCommandBuffers(g->vd->act_dev, &ai, &g->command));
    VkExportSemaphoreCreateInfo ei = {
        .sType = VK_STRUCTURE_TYPE_EXPORT_SEMAPHORE_CREATE_INFO,
        .handleTypes = VK_EXTERNAL_SEMAPHORE_HANDLE_TYPE_OPAQUE_FD_BIT};
    VkSemaphoreCreateInfo si = {
        .sType = VK_STRUCTURE_TYPE_SEMAPHORE_CREATE_INFO, .pNext = &ei};
    VK(g, vkCreateSemaphore(g->vd->act_dev, &si, NULL, &g->semaphore));
    PFN_vkGetSemaphoreFdKHR get_sem =
        (void *)vkGetDeviceProcAddr(g->vd->act_dev, "vkGetSemaphoreFdKHR");
    REQUIRE(g, get_sem);
    VkSemaphoreGetFdInfoKHR sf = {
        .sType = VK_STRUCTURE_TYPE_SEMAPHORE_GET_FD_INFO_KHR,
        .semaphore = g->semaphore,
        .handleType = VK_EXTERNAL_SEMAPHORE_HANDLE_TYPE_OPAQUE_FD_BIT};
    VK(g, get_sem(g->vd->act_dev, &sf, &fd));
    g->create_semaphores(1, &g->gl_sem);
    g->import_semaphore(g->gl_sem, GL_HANDLE_TYPE_OPAQUE_FD_EXT, fd);
    REQUIRE(g, glGetError() == GL_NO_ERROR);
    VkFenceCreateInfo fi = {.sType = VK_STRUCTURE_TYPE_FENCE_CREATE_INFO};
    VK(g, vkCreateFence(g->vd->act_dev, &fi, NULL, &g->fence));
    const char *vertex =
        "#version 300 es\nprecision highp float;uniform vec4 rect;uniform vec2 "
        "screen;out vec2 uv;void main(){vec2 "
        "p=vec2((gl_VertexID==1||gl_VertexID==3)?1.:0.,gl_VertexID>=2?1.:0.);"
        "uv="
        "p;vec2 xy=rect.xy+p*rect.zw;gl_Position=vec4(xy/screen*2.-1.,0.,1.);}";
    const char *fragment =
        "#version 300 es\nprecision highp float;uniform sampler2D image;in "
        "vec2 "
        "uv;out vec4 color;void main(){color=texture(image,uv);}";
    GLuint vs = 0, fs = 0;
    int result = shader(g, GL_VERTEX_SHADER, vertex, &vs);
    if (result == 0)
        result = shader(g, GL_FRAGMENT_SHADER, fragment, &fs);
    if (result == 0) {
        g->program = glCreateProgram();
        glAttachShader(g->program, vs);
        glAttachShader(g->program, fs);
        glLinkProgram(g->program);
        GLint ok;
        glGetProgramiv(g->program, GL_LINK_STATUS, &ok);
        if (!ok) {
            glGetProgramInfoLog(g->program, sizeof g->error, NULL, g->error);
            result = -1;
        }
    }
    glDeleteShader(vs);
    glDeleteShader(fs);
    if (result < 0)
        return -1;
    glGenTextures(1, &g->cursor);
    glGenTextures(1, &g->background);
    return 0;
}

BsGpu *bs_gpu_open(const char *node, int width, int height, char *error,
                   int error_size) {
    if (width <= 0 || height <= 0 || width > 8192 || height > 8192 ||
        (int64_t)width * height > 16777216 || !node) {
        snprintf(error, error_size, "invalid compositor dimensions/device");
        return NULL;
    }
    BsGpu *g = calloc(1, sizeof(*g));
    if (!g) {
        snprintf(error, error_size, "allocate compositor");
        return NULL;
    }
    g->node = -1;
    g->width = width;
    g->height = height;
    if (initialize(g, node) < 0) {
        snprintf(error, error_size, "%s", g->error);
        bs_gpu_close(g);
        return NULL;
    }
    return g;
}
const char *bs_gpu_error(BsGpu *g) { return g->error; }

void bs_gpu_close(BsGpu *g) {
    if (!g)
        return;
    if (g->context != EGL_NO_CONTEXT) {
        eglMakeCurrent(g->display, EGL_NO_SURFACE, EGL_NO_SURFACE, g->context);
        glFinish();
        if (g->vd)
            vkDeviceWaitIdle(g->vd->act_dev);
        glDeleteFramebuffers(1, &g->framebuffer);
        glDeleteProgram(g->program);
        glDeleteTextures(1, &g->background);
        glDeleteTextures(1, &g->cursor);
        glDeleteTextures(1, &g->output);
        if (g->background_image && g->destroy_image)
            g->destroy_image(g->display, g->background_image);
        if (g->memory && g->delete_memory)
            g->delete_memory(1, &g->memory);
        if (g->gl_sem && g->delete_semaphores)
            g->delete_semaphores(1, &g->gl_sem);
    }
    if (g->vd) {
        if (g->fence)
            vkDestroyFence(g->vd->act_dev, g->fence, NULL);
        if (g->semaphore)
            vkDestroySemaphore(g->vd->act_dev, g->semaphore, NULL);
        if (g->pool)
            vkDestroyCommandPool(g->vd->act_dev, g->pool, NULL);
    }
    av_packet_free(&g->packet);
    avcodec_free_context(&g->encoder);
    avfilter_graph_free(&g->graph);
    if (g->mux) {
        g->mux->pb = NULL;
        avformat_free_context(g->mux);
    }
    if (g->io)
        av_freep(&g->io->buffer);
    avio_context_free(&g->io);
    av_frame_free(&g->frame);
    av_buffer_unref(&g->frames);
    av_buffer_unref(&g->device);
    if (g->display != EGL_NO_DISPLAY) {
        eglMakeCurrent(g->display, EGL_NO_SURFACE, EGL_NO_SURFACE,
                       EGL_NO_CONTEXT);
        if (g->context != EGL_NO_CONTEXT)
            eglDestroyContext(g->display, g->context);
        eglTerminate(g->display);
    }
    if (g->gbm)
        gbm_device_destroy(g->gbm);
    if (g->node >= 0)
        close(g->node);
    free(g);
}
int bs_gpu_rgba_background(BsGpu *g, const uint8_t *rgba) {
    REQUIRE(g, rgba);
    if (g->background_image) {
        g->destroy_image(g->display, g->background_image);
        g->background_image = EGL_NO_IMAGE_KHR;
        glDeleteTextures(1, &g->background);
        glGenTextures(1, &g->background);
    }
    glBindTexture(GL_TEXTURE_2D, g->background);
    texture_parameters();
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, g->width, g->height, 0, GL_RGBA,
                 GL_UNSIGNED_BYTE, rgba);
    REQUIRE(g, glGetError() == GL_NO_ERROR);
    return 0;
}
int bs_gpu_background(BsGpu *g, int width, int height, uint32_t fourcc,
                      uint64_t modifier, int planes, const int *fds,
                      const uint32_t *strides, const uint32_t *offsets) {
    REQUIRE(g, width == g->width && height == g->height && planes >= 1 &&
                   planes <= 4);
    const EGLint fd_keys[] = {
        EGL_DMA_BUF_PLANE0_FD_EXT, EGL_DMA_BUF_PLANE1_FD_EXT,
        EGL_DMA_BUF_PLANE2_FD_EXT, EGL_DMA_BUF_PLANE3_FD_EXT};
    const EGLint pitch_keys[] = {
        EGL_DMA_BUF_PLANE0_PITCH_EXT, EGL_DMA_BUF_PLANE1_PITCH_EXT,
        EGL_DMA_BUF_PLANE2_PITCH_EXT, EGL_DMA_BUF_PLANE3_PITCH_EXT};
    const EGLint offset_keys[] = {
        EGL_DMA_BUF_PLANE0_OFFSET_EXT, EGL_DMA_BUF_PLANE1_OFFSET_EXT,
        EGL_DMA_BUF_PLANE2_OFFSET_EXT, EGL_DMA_BUF_PLANE3_OFFSET_EXT};
    const EGLint lo_keys[] = {
        EGL_DMA_BUF_PLANE0_MODIFIER_LO_EXT, EGL_DMA_BUF_PLANE1_MODIFIER_LO_EXT,
        EGL_DMA_BUF_PLANE2_MODIFIER_LO_EXT, EGL_DMA_BUF_PLANE3_MODIFIER_LO_EXT};
    const EGLint hi_keys[] = {
        EGL_DMA_BUF_PLANE0_MODIFIER_HI_EXT, EGL_DMA_BUF_PLANE1_MODIFIER_HI_EXT,
        EGL_DMA_BUF_PLANE2_MODIFIER_HI_EXT, EGL_DMA_BUF_PLANE3_MODIFIER_HI_EXT};
    EGLint attrs[47] = {
        EGL_WIDTH,     width, EGL_HEIGHT, height, EGL_LINUX_DRM_FOURCC_EXT,
        (EGLint)fourcc};
    int n = 6;
    for (int i = 0; i < planes; i++) {
        REQUIRE(g, fds[i] >= 0 && strides[i] > 0 && strides[i] <= INT32_MAX &&
                       offsets[i] <= INT32_MAX);
        attrs[n++] = fd_keys[i];
        attrs[n++] = fds[i];
        attrs[n++] = pitch_keys[i];
        attrs[n++] = (EGLint)strides[i];
        attrs[n++] = offset_keys[i];
        attrs[n++] = (EGLint)offsets[i];
        if (modifier != 0x00ffffffffffffffULL) {
            attrs[n++] = lo_keys[i];
            attrs[n++] = (EGLint)modifier;
            attrs[n++] = hi_keys[i];
            attrs[n++] = (EGLint)(modifier >> 32);
        }
    }
    attrs[n] = EGL_NONE;
    EGLImageKHR image = g->create_image(g->display, EGL_NO_CONTEXT,
                                        EGL_LINUX_DMA_BUF_EXT, NULL, attrs);
    REQUIRE(g, image != EGL_NO_IMAGE_KHR);
    if (g->background_image)
        g->destroy_image(g->display, g->background_image);
    g->background_image = image;
    glBindTexture(GL_TEXTURE_2D, g->background);
    g->target_image(GL_TEXTURE_2D, image);
    texture_parameters();
    REQUIRE(g, glGetError() == GL_NO_ERROR);
    return 0;
}
int bs_gpu_cursor(BsGpu *g, int width, int height, const uint8_t *rgba) {
    REQUIRE(g,
            width > 0 && height > 0 && width <= 1024 && height <= 1024 && rgba);
    glBindTexture(GL_TEXTURE_2D, g->cursor);
    texture_parameters();
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA8, width, height, 0, GL_RGBA,
                 GL_UNSIGNED_BYTE, rgba);
    REQUIRE(g, glGetError() == GL_NO_ERROR);
    g->cursor_width = width;
    g->cursor_height = height;
    return 0;
}
/* FFmpeg 8.1 exposes queue locks for external users. Keep the version-dependent
 * compatibility calls together, rather than ignoring synchronization warnings.
 */
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wdeprecated-declarations"
static VkResult submit(BsGpu *g, VkSubmitInfo *info) {
    g->vd->lock_queue(g->dc, g->family, 0);
    VkResult result = vkQueueSubmit(g->queue, 1, info, g->fence);
    g->vd->unlock_queue(g->dc, g->family, 0);
    return result;
}
#pragma GCC diagnostic pop
static int handoff(BsGpu *g, int acquire) {
    AVVkFrame *vf = (void *)g->frame->data[0];
    AVHWFramesContext *fc = (void *)g->frames->data;
    AVVulkanFramesContext *vfc = fc->hwctx;
    VK(g, vkResetFences(g->vd->act_dev, 1, &g->fence));
    VK(g, vkResetCommandBuffer(g->command, 0));
    VkCommandBufferBeginInfo bi = {
        .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO};
    VK(g, vkBeginCommandBuffer(g->command, &bi));
    vfc->lock_frame(fc, vf);
    VkImageMemoryBarrier barrier = {
        .sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
        .srcAccessMask = acquire ? 0 : vf->access[0],
        .dstAccessMask = acquire ? VK_ACCESS_MEMORY_READ_BIT : 0,
        .oldLayout = acquire ? VK_IMAGE_LAYOUT_GENERAL : vf->layout[0],
        .newLayout = VK_IMAGE_LAYOUT_GENERAL,
        .srcQueueFamilyIndex = acquire ? VK_QUEUE_FAMILY_EXTERNAL : g->family,
        .dstQueueFamilyIndex = acquire ? g->family : VK_QUEUE_FAMILY_EXTERNAL,
        .image = vf->img[0],
        .subresourceRange = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1}};
    vkCmdPipelineBarrier(g->command, VK_PIPELINE_STAGE_ALL_COMMANDS_BIT,
                         VK_PIPELINE_STAGE_ALL_COMMANDS_BIT, 0, 0, NULL, 0,
                         NULL, 1, &barrier);
    VkResult result = vkEndCommandBuffer(g->command);
    uint64_t zero = 0, next = vf->sem_value[0] + 1;
    VkTimelineSemaphoreSubmitInfo ti = {
        .sType = VK_STRUCTURE_TYPE_TIMELINE_SEMAPHORE_SUBMIT_INFO,
        .waitSemaphoreValueCount = 1,
        .pWaitSemaphoreValues = acquire ? &zero : &vf->sem_value[0],
        .signalSemaphoreValueCount = 1,
        .pSignalSemaphoreValues = acquire ? &next : &zero};
    VkPipelineStageFlags stage = VK_PIPELINE_STAGE_ALL_COMMANDS_BIT;
    VkSubmitInfo info = {
        .sType = VK_STRUCTURE_TYPE_SUBMIT_INFO,
        .pNext = &ti,
        .waitSemaphoreCount = 1,
        .pWaitSemaphores = acquire ? &g->semaphore : &vf->sem[0],
        .pWaitDstStageMask = &stage,
        .commandBufferCount = 1,
        .pCommandBuffers = &g->command,
        .signalSemaphoreCount = 1,
        .pSignalSemaphores = acquire ? &vf->sem[0] : &g->semaphore};
    if (result == VK_SUCCESS)
        result = submit(g, &info);
    if (result == VK_SUCCESS && acquire) {
        vf->sem_value[0] = next;
        vf->layout[0] = VK_IMAGE_LAYOUT_GENERAL;
        vf->access[0] = VK_ACCESS_MEMORY_READ_BIT;
    }
    vfc->unlock_frame(fc, vf);
    if (result != VK_SUCCESS)
        FAIL(g, "GPU handoff submission: %d", result);
    VK(g,
       vkWaitForFences(g->vd->act_dev, 1, &g->fence, VK_TRUE, 5000000000ULL));
    return 0;
}
int bs_gpu_draw(BsGpu *g, int visible, float x, float y) {
    if (handoff(g, 0) < 0)
        return -1;
    GLenum layout = GL_LAYOUT_GENERAL_EXT;
    g->wait_semaphore(g->gl_sem, 0, NULL, 1, &g->output, &layout);
    glBindFramebuffer(GL_FRAMEBUFFER, g->framebuffer);
    glViewport(0, 0, g->width, g->height);
    glUseProgram(g->program);
    glUniform2f(glGetUniformLocation(g->program, "screen"), g->width,
                g->height);
    glActiveTexture(GL_TEXTURE0);
    glUniform1i(glGetUniformLocation(g->program, "image"), 0);
    glDisable(GL_BLEND);
    glBindTexture(GL_TEXTURE_2D, g->background);
    glUniform4f(glGetUniformLocation(g->program, "rect"), 0, 0, g->width,
                g->height);
    glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    if (visible && g->cursor_width > 0) {
        glEnable(GL_BLEND);
        glBlendFunc(GL_ONE, GL_ONE_MINUS_SRC_ALPHA);
        glBindTexture(GL_TEXTURE_2D, g->cursor);
        glUniform4f(glGetUniformLocation(g->program, "rect"), x, y,
                    g->cursor_width, g->cursor_height);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        glDisable(GL_BLEND);
    }
    g->signal_semaphore(g->gl_sem, 0, NULL, 1, &g->output, &layout);
    glFlush();
    REQUIRE(g, glGetError() == GL_NO_ERROR);
    return handoff(g, 1);
}
int bs_gpu_readback(BsGpu *g, uint8_t *rgba, int bytes) {
    REQUIRE(g, rgba && bytes > 0 &&
                   (int64_t)bytes == (int64_t)g->width * g->height * 4);
    AVFrame *cpu = av_frame_alloc();
    REQUIRE(g, cpu);
    int result = av_hwframe_transfer_data(cpu, g->frame, 0);
    if (result >= 0 && cpu->format == AV_PIX_FMT_RGBA) {
        for (int y = 0; y < g->height; y++)
            memcpy(rgba + (size_t)y * g->width * 4,
                   cpu->data[0] + (size_t)y * cpu->linesize[0],
                   (size_t)g->width * 4);
    } else if (result >= 0)
        result = AVERROR(EINVAL);
    av_frame_free(&cpu);
    AV(g, result);
    return 0;
}

static int write_packet(void *opaque, const uint8_t *buffer, int size) {
    BsGpu *g = opaque;
    int done = 0;
    while (done < size) {
        ssize_t n = write(g->output_fd, buffer + done, (size_t)(size - done));
        if (n < 0 && errno == EINTR)
            continue;
        if (n <= 0)
            return AVERROR(n < 0 ? errno : EIO);
        done += (int)n;
    }
    return done;
}
int bs_gpu_encoder(BsGpu *g, const char *codec, int fps, int fd) {
    REQUIRE(g, !g->encoder && fps >= 1 && fps <= 240 && fd >= 0 && codec);
    int cpu = !strcmp(codec, "libx264");
    REQUIRE(g, cpu || !strcmp(codec, "h264_vulkan"));
    const AVCodec *implementation = avcodec_find_encoder_by_name(codec);
    REQUIRE(g, implementation);
    g->graph = avfilter_graph_alloc();
    REQUIRE(g, g->graph);
    char args[256];
    snprintf(args, sizeof args,
             "video_size=%dx%d:pix_fmt=%d:time_base=1/%d:pixel_aspect=1/1",
             g->width, g->height, AV_PIX_FMT_VULKAN, fps);
    g->source = avfilter_graph_alloc_filter(
        g->graph, avfilter_get_by_name("buffer"), "source");
    REQUIRE(g, g->source);
    AVBufferSrcParameters *params = av_buffersrc_parameters_alloc();
    REQUIRE(g, params);
    params->hw_frames_ctx = av_buffer_ref(g->frames);
    params->color_space = AVCOL_SPC_BT709;
    params->color_range = AVCOL_RANGE_JPEG;
    int result = av_buffersrc_parameters_set(g->source, params);
    av_buffer_unref(&params->hw_frames_ctx);
    av_free(params);
    AV(g, result);
    AV(g, avfilter_init_str(g->source, args));
    AVFilterContext *scale = NULL;
    AV(g, avfilter_graph_create_filter(
              &scale, avfilter_get_by_name("scale_vulkan"), "nv12",
              "format=nv12:out_range=limited", NULL, g->graph));
    AV(g, avfilter_graph_create_filter(&g->sink,
                                       avfilter_get_by_name("buffersink"),
                                       "sink", NULL, NULL, g->graph));
    AV(g, avfilter_link(g->source, 0, scale, 0));
    AV(g, avfilter_link(scale, 0, g->sink, 0));
    AV(g, avfilter_graph_config(g->graph, NULL));
    g->encoder = avcodec_alloc_context3(implementation);
    REQUIRE(g, g->encoder);
    g->encoder->width = g->width;
    g->encoder->height = g->height;
    g->encoder->time_base = (AVRational){1, fps};
    g->encoder->framerate = (AVRational){fps, 1};
    g->encoder->pix_fmt = cpu ? AV_PIX_FMT_NV12 : AV_PIX_FMT_VULKAN;
    g->encoder->gop_size = fps;
    g->encoder->max_b_frames = 0;
    g->encoder->flags |= AV_CODEC_FLAG_CLOSED_GOP | AV_CODEC_FLAG_GLOBAL_HEADER;
    g->encoder->color_range = AVCOL_RANGE_MPEG;
    g->encoder->colorspace = AVCOL_SPC_BT709;
    g->encoder->color_primaries = AVCOL_PRI_BT709;
    g->encoder->color_trc = AVCOL_TRC_BT709;
    if (!cpu)
        g->encoder->hw_frames_ctx =
            av_buffer_ref(av_buffersink_get_hw_frames_ctx(g->sink));
    AVDictionary *options = NULL;
    if (cpu) {
        av_dict_set(&options, "preset", "veryfast", 0);
        av_dict_set(&options, "crf", "16", 0);
        av_dict_set(&options, "tune", "zerolatency", 0);
    } else {
        av_dict_set(&options, "qp", "16", 0);
        av_dict_set(&options, "rc_mode", "cqp", 0);
        av_dict_set(&options, "async_depth", "2", 0);
    }
    result = avcodec_open2(g->encoder, implementation, &options);
    int unused = av_dict_count(options);
    av_dict_free(&options);
    AV(g, result);
    REQUIRE(g, unused == 0);
    AV(g, avformat_alloc_output_context2(&g->mux, NULL, "nut", NULL));
    AVStream *stream = avformat_new_stream(g->mux, NULL);
    REQUIRE(g, stream);
    stream->time_base = g->encoder->time_base;
    stream->avg_frame_rate = g->encoder->framerate;
    AV(g, avcodec_parameters_from_context(stream->codecpar, g->encoder));
    unsigned char *buffer = av_malloc(32768);
    REQUIRE(g, buffer);
    g->output_fd = fd;
    g->io = avio_alloc_context(buffer, 32768, 1, g, NULL, write_packet, NULL);
    if (!g->io) {
        av_free(buffer);
        FAIL(g, "allocate output IO");
    }
    g->mux->pb = g->io;
    g->mux->flags |= AVFMT_FLAG_CUSTOM_IO | AVFMT_FLAG_FLUSH_PACKETS;
    av_dict_set(&options, "write_index", "0", 0);
    av_dict_set(&options, "syncpoints", "none", 0);
    av_dict_set(&options, "strict", "experimental", 0);
    av_dict_set(&options, "flush_packets", "1", 0);
    result = avformat_write_header(g->mux, &options);
    unused = av_dict_count(options);
    av_dict_free(&options);
    AV(g, result);
    REQUIRE(g, unused == 0);
    g->header_written = 1;
    g->packet = av_packet_alloc();
    REQUIRE(g, g->packet);
    return 0;
}
static int packets(BsGpu *g) {
    for (;;) {
        int result = avcodec_receive_packet(g->encoder, g->packet);
        if (result == AVERROR(EAGAIN) || result == AVERROR_EOF)
            return 0;
        AV(g, result);
        if (!g->packet->duration)
            g->packet->duration = 1;
        av_packet_rescale_ts(g->packet, g->encoder->time_base,
                             g->mux->streams[0]->time_base);
        g->packet->stream_index = 0;
        result = av_interleaved_write_frame(g->mux, g->packet);
        av_packet_unref(g->packet);
        AV(g, result);
    }
}
/* Publish compute writes through a graphics-capable queue before handing frames
 * to a dedicated transfer/video queue. Specific SHADER_WRITE access masks are
 * invalid on those queues, even when ALL_COMMANDS is requested. */
static int readable(BsGpu *g, AVFrame *frame) {
    AVHWFramesContext *fc = (void *)frame->hw_frames_ctx->data;
    AVVulkanFramesContext *vfc = fc->hwctx;
    AVVkFrame *vf = (void *)frame->data[0];
    VkImageMemoryBarrier barriers[AV_NUM_DATA_POINTERS];
    VkPipelineStageFlags stages[AV_NUM_DATA_POINTERS];
    uint64_t values[AV_NUM_DATA_POINTERS];
    unsigned n = 0;
    VK(g, vkResetFences(g->vd->act_dev, 1, &g->fence));
    VK(g, vkResetCommandBuffer(g->command, 0));
    VkCommandBufferBeginInfo bi = {
        .sType = VK_STRUCTURE_TYPE_COMMAND_BUFFER_BEGIN_INFO};
    VK(g, vkBeginCommandBuffer(g->command, &bi));
    vfc->lock_frame(fc, vf);
    for (; n < AV_NUM_DATA_POINTERS && vf->img[n]; n++) {
        barriers[n] = (VkImageMemoryBarrier){
            .sType = VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER,
            .srcAccessMask = vf->access[n],
            .dstAccessMask = VK_ACCESS_MEMORY_READ_BIT,
            .oldLayout = vf->layout[n],
            .newLayout = VK_IMAGE_LAYOUT_GENERAL,
            .srcQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
            .dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED,
            .image = vf->img[n],
            .subresourceRange = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1}};
        stages[n] = VK_PIPELINE_STAGE_ALL_COMMANDS_BIT;
        values[n] = vf->sem_value[n] + 1;
    }
    vkCmdPipelineBarrier(g->command, VK_PIPELINE_STAGE_ALL_COMMANDS_BIT,
                         VK_PIPELINE_STAGE_ALL_COMMANDS_BIT, 0, 0, NULL, 0,
                         NULL, n, barriers);
    VkResult result = vkEndCommandBuffer(g->command);
    VkTimelineSemaphoreSubmitInfo ti = {
        .sType = VK_STRUCTURE_TYPE_TIMELINE_SEMAPHORE_SUBMIT_INFO,
        .waitSemaphoreValueCount = n,
        .pWaitSemaphoreValues = vf->sem_value,
        .signalSemaphoreValueCount = n,
        .pSignalSemaphoreValues = values};
    VkSubmitInfo info = {.sType = VK_STRUCTURE_TYPE_SUBMIT_INFO,
                         .pNext = &ti,
                         .waitSemaphoreCount = n,
                         .pWaitSemaphores = vf->sem,
                         .pWaitDstStageMask = stages,
                         .commandBufferCount = 1,
                         .pCommandBuffers = &g->command,
                         .signalSemaphoreCount = n,
                         .pSignalSemaphores = vf->sem};
    if (result == VK_SUCCESS)
        result = submit(g, &info);
    if (result == VK_SUCCESS)
        for (unsigned i = 0; i < n; i++) {
            vf->sem_value[i] = values[i];
            vf->layout[i] = VK_IMAGE_LAYOUT_GENERAL;
            vf->access[i] = VK_ACCESS_MEMORY_READ_BIT;
        }
    vfc->unlock_frame(fc, vf);
    if (result != VK_SUCCESS)
        FAIL(g, "publish converted frame: %d", result);
    VK(g,
       vkWaitForFences(g->vd->act_dev, 1, &g->fence, VK_TRUE, 5000000000ULL));
    return 0;
}
int bs_gpu_encode(BsGpu *g, int64_t pts) {
    REQUIRE(g, g->encoder && !g->finished);
    g->frame->pts = pts;
    g->frame->color_range = AVCOL_RANGE_JPEG;
    g->frame->colorspace = AVCOL_SPC_BT709;
    g->frame->color_primaries = AVCOL_PRI_BT709;
    g->frame->color_trc = AVCOL_TRC_BT709;
    AV(g, av_buffersrc_add_frame_flags(g->source, g->frame,
                                       AV_BUFFERSRC_FLAG_KEEP_REF));
    AVFrame *filtered = av_frame_alloc(), *cpu = NULL;
    if (!filtered)
        FAIL(g, "allocate filtered frame");
    int result = av_buffersink_get_frame(g->sink, filtered);
    AVFrame *input = filtered;
    if (result >= 0 && readable(g, filtered) < 0) {
        av_frame_free(&filtered);
        return -1;
    }
    if (result >= 0 && g->encoder->pix_fmt != AV_PIX_FMT_VULKAN) {
        cpu = av_frame_alloc();
        result =
            cpu ? av_hwframe_transfer_data(cpu, filtered, 0) : AVERROR(ENOMEM);
        if (result >= 0) {
            cpu->pts = pts;
            input = cpu;
        }
    }
    if (result >= 0)
        result = avcodec_send_frame(g->encoder, input);
    av_frame_free(&cpu);
    av_frame_free(&filtered);
    AV(g, result);
    return packets(g);
}
int bs_gpu_finish(BsGpu *g) {
    REQUIRE(g, g->encoder && !g->finished);
    AV(g, avcodec_send_frame(g->encoder, NULL));
    if (packets(g) < 0)
        return -1;
    AV(g, av_write_trailer(g->mux));
    avio_flush(g->io);
    AV(g, g->io->error);
    g->finished = 1;
    return 0;
}
