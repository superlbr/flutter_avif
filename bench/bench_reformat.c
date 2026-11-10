// YUV→RGBA（avifImageYUVToRGB）单点 bench。
// 用同一张 AVIF、同一份 YUV 平面，反复跑 reformat，只量这一步；
// 顺便把 RGBA 结果落盘，用来对比两条路径（标量 C / libyuv）的像素差。
#include <avif/avif.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

static double now_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (double)ts.tv_sec * 1000.0 + (double)ts.tv_nsec / 1e6;
}

static int cmp_double(const void* a, const void* b) {
    double x = *(const double*)a, y = *(const double*)b;
    return (x > y) - (x < y);
}

static unsigned char* read_file(const char* path, size_t* out_size) {
    FILE* f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long n = ftell(f);
    fseek(f, 0, SEEK_SET);
    unsigned char* buf = (unsigned char*)malloc((size_t)n);
    if (fread(buf, 1, (size_t)n, f) != (size_t)n) { fclose(f); free(buf); return NULL; }
    fclose(f);
    *out_size = (size_t)n;
    return buf;
}

int main(int argc, char** argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: %s <in.avif|synth:WxH> [out.rgba] [iters]\n", argv[0]);
        return 2;
    }
    const char* out_path = (argc > 2 && argv[2][0]) ? argv[2] : NULL;
    int iters = argc > 3 ? atoi(argv[3]) : 50;

    avifImage* img = NULL;
    avifDecoder* dec = NULL;
    unsigned char* data = NULL;
    double decode_ms = -1.0;

    if (strncmp(argv[1], "synth:", 6) == 0) {
        // 合成 YUV420 8bit 图：手边没有 585 宽的 AVIF 样本，又不想为了 bench 引编码器，
        // 直接构造 avifImage 也能量到同一段 YUV→RGBA 的耗时。
        unsigned w = 0, h = 0;
        if (sscanf(argv[1] + 6, "%ux%u", &w, &h) != 2 || !w || !h) {
            fprintf(stderr, "bad synth size\n");
            return 2;
        }
        img = avifImageCreate(w, h, 8, AVIF_PIXEL_FORMAT_YUV420);
        if (!img || avifImageAllocatePlanes(img, AVIF_PLANES_YUV) != AVIF_RESULT_OK) {
            fprintf(stderr, "avifImageCreate failed\n");
            return 1;
        }
        for (uint32_t p = 0; p < 3; p++) {
            uint32_t ph = avifImagePlaneHeight(img, p);
            for (uint32_t y = 0; y < ph; y++) {
                uint8_t* row = avifImagePlane(img, p) + (size_t)y * avifImagePlaneRowBytes(img, p);
                for (uint32_t x = 0; x < avifImagePlaneWidth(img, p); x++) row[x] = (uint8_t)(x + y * 3);
            }
        }
        printf("file=%s (synth) size=%ux%u depth=%u yuv=%d\n", argv[1], img->width, img->height,
               img->depth, (int)img->yuvFormat);
    } else {
        size_t size = 0;
        data = read_file(argv[1], &size);
        if (!data) {
            fprintf(stderr, "cannot read %s\n", argv[1]);
            return 1;
        }

        dec = avifDecoderCreate();
        if (!dec) {
            fprintf(stderr, "avifDecoderCreate failed\n");
            return 1;
        }
        avifResult r = avifDecoderSetIOMemory(dec, data, size);
        if (r != AVIF_RESULT_OK) {
            fprintf(stderr, "SetIOMemory: %s\n", avifResultToString(r));
            return 1;
        }
        r = avifDecoderParse(dec);
        if (r != AVIF_RESULT_OK) {
            fprintf(stderr, "Parse: %s\n", avifResultToString(r));
            return 1;
        }

        double t0 = now_ms();
        r = avifDecoderNextImage(dec);
        decode_ms = now_ms() - t0;
        if (r != AVIF_RESULT_OK) {
            fprintf(stderr, "NextImage: %s\n", avifResultToString(r));
            return 1;
        }
        img = dec->image;
        printf("file=%s size=%ux%u depth=%u yuv=%d\n", argv[1], img->width, img->height, img->depth,
               (int)img->yuvFormat);
    }

    avifRGBImage rgb;
    avifRGBImageSetDefaults(&rgb, img);
    rgb.format = AVIF_RGB_FORMAT_RGBA;
    rgb.depth = 8;
    rgb.alphaPremultiplied = AVIF_TRUE;
    if (avifRGBImageAllocatePixels(&rgb) != AVIF_RESULT_OK) {
        fprintf(stderr, "alloc failed\n");
        return 1;
    }

    avifResult r = AVIF_RESULT_OK;
    double* ts = (double*)malloc(sizeof(double) * (size_t)iters);
    for (int i = 0; i < iters; i++) {
        double s = now_ms();
        r = avifImageYUVToRGB(img, &rgb);
        ts[i] = now_ms() - s;
        if (r != AVIF_RESULT_OK) {
            fprintf(stderr, "YUVToRGB: %s\n", avifResultToString(r));
            return 1;
        }
    }
    qsort(ts, (size_t)iters, sizeof(double), cmp_double);
    double sum = 0;
    for (int i = 0; i < iters; i++) sum += ts[i];
    if (decode_ms >= 0) printf("decode(parse+next)=%.2f ms\n", decode_ms);
    printf("reformat median=%.2f ms mean=%.2f min=%.2f max=%.2f n=%d  => %.1f Mpx/s\n",
           ts[iters / 2], sum / iters, ts[0], ts[iters - 1], iters,
           ((double)img->width * img->height * iters) / (sum * 1000.0));

    if (out_path) {
        FILE* o = fopen(out_path, "wb");
        if (!o) {
            fprintf(stderr, "cannot write %s\n", out_path);
            return 1;
        }
        fwrite(rgb.pixels, 1, (size_t)rgb.rowBytes * img->height, o);
        fclose(o);
        printf("wrote %s (%d bytes/row × %u rows)\n", out_path, rgb.rowBytes, img->height);
    }

    avifRGBImageFreePixels(&rgb);
    // 真实文件路径下 img 就是 dec->image，由 decoder 持有；
    // 先 destroy decoder 再 destroy img 会双重释放（进程在退出前 SIGTRAP）。
    if (dec) {
        avifDecoderDestroy(dec);
    } else if (img) {
        avifImageDestroy(img);
    }
    free(data);
    free(ts);
    return 0;
}
