/*
 * A minimal FFGL 2 host for testing the plugin without Resolume.
 *
 * Makes a headless OpenGL 4.1 core context, loads the plugin binary, queries its info
 * and parameters, then renders generated textures through FF_PROCESS_OPENGL into a
 * framebuffer and checks the pixels and the GL state the plugin leaves behind.
 *
 *   minihost <plugin binary> [case]   cases: all (default), info, passthrough,
 *                                     brightness, padding, blur, vignette, state, timing
 * Exit status 0 when every check passes.
 */
#define GL_SILENCE_DEPRECATION
#include <OpenGL/OpenGL.h>
#include <OpenGL/gl3.h>
#include <dlfcn.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

/* The FFGL 2.1 ABI (from Resolume's FFGL.h). */
typedef union { uint32_t u; void *p; } FFMixed;
typedef FFMixed (*PlugMain)(uint32_t, FFMixed, void *);
typedef struct { uint32_t major, minor; char id[4]; char name[16]; uint32_t type; } PluginInfo;
typedef struct { uint32_t number; FFMixed value; } SetParameter;
typedef struct { uint32_t number; float min, max; } GetRange;
typedef struct { uint32_t number; char *address; uint32_t max; } GetString;
typedef struct { uint32_t param, element; } GetElement;
typedef struct { GLuint x, y, w, h; } Viewport;
typedef struct { uint32_t w, h, hw, hh; GLuint handle; } Texture;
typedef struct { uint32_t count; Texture **textures; GLuint fbo; } Process;

enum {
    GET_INFO = 0, INITIALISE_V2 = 34, DEINITIALISE = 2, GET_NUM_PARAMETERS = 4, GET_PARAMETER_NAME = 5,
    GET_PARAMETER_DEFAULT = 6, GET_PARAMETER_DISPLAY = 7, SET_PARAMETER = 8, GET_PARAMETER = 9,
    GET_PLUGIN_CAPS = 10, GET_PARAMETER_TYPE = 15, PROCESS_OPENGL = 17, INSTANTIATE_GL = 18,
    DEINSTANTIATE_GL = 19, SET_TIME = 20, GET_NUM_PARAMETER_ELEMENTS = 31, GET_PARAMETER_ELEMENT_NAME = 35,
    GET_RANGE = 41, GET_PARAM_GROUP = 50,
};
#define FF_FAIL 0xFFFFFFFFu
#define TYPE_OPTION 11u

static PlugMain plug;
static int failures = 0;
#define CHECK(cond, ...) do { if (!(cond)) { failures++; printf("  FAIL: "); printf(__VA_ARGS__); printf("\n"); } } while (0)

static FFMixed u(uint32_t v) { FFMixed m = {0}; m.u = v; return m; }
static FFMixed ptr(void *p) { FFMixed m; m.p = p; return m; }
static FFMixed call(uint32_t code, FFMixed in, void *inst) { return plug(code, in, inst); }

/* Parameter index by name, from the plugin's own table. */
static int param(const char *name) {
    uint32_t n = call(GET_NUM_PARAMETERS, u(0), NULL).u;
    for (uint32_t i = 0; i < n; i++) {
        const char *p = call(GET_PARAMETER_NAME, u(i), NULL).p;
        if (p && !strcmp(p, name)) return (int)i;
    }
    failures++;
    printf("  FAIL: no parameter %s\n", name);
    return 0;
}

static void set(void *inst, const char *name, float v) {
    SetParameter s;
    s.number = (uint32_t)param(name);
    memset(&s.value, 0, sizeof s.value);
    memcpy(&s.value.u, &v, 4);
    CHECK(call(SET_PARAMETER, ptr(&s), inst).u == 0, "set %s failed", name);
}

/* ---- GL helpers -------------------------------------------------------------------- */

static GLuint make_texture(int w, int h, const float *pixels) {
    GLuint t;
    glGenTextures(1, &t);
    glBindTexture(GL_TEXTURE_2D, t);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
    glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA32F, w, h, 0, GL_RGBA, GL_FLOAT, pixels);
    glBindTexture(GL_TEXTURE_2D, 0);
    return t;
}

typedef struct { GLuint fbo, color; int w, h; } Target;

static Target make_target(int w, int h) {
    Target t = {0, make_texture(w, h, NULL), w, h};
    glGenFramebuffers(1, &t.fbo);
    glBindFramebuffer(GL_FRAMEBUFFER, t.fbo);
    glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t.color, 0);
    if (glCheckFramebufferStatus(GL_FRAMEBUFFER) != GL_FRAMEBUFFER_COMPLETE) { fprintf(stderr, "FBO incomplete\n"); exit(2); }
    return t;
}

/* Pixel (x, y), y up, of an RGBA float image `w` wide. */
static float *px(float *img, int w, int x, int y) { return &img[((size_t)y * w + x) * 4]; }

/* Straight-alpha pattern: red ramps with x, green with y, blue checker, alpha 1 with a
   half-transparent band on the right. Stored premultiplied, as FFGL textures are. */
static void pattern(float *straight, float *premult, int w, int h, int stride) {
    for (int y = 0; y < h; y++)
        for (int x = 0; x < w; x++) {
            float *s = px(straight, w, x, y), *p = px(premult, stride, x, y);
            s[0] = (float)x / (w - 1);
            s[1] = (float)y / (h - 1);
            s[2] = ((x / 8 + y / 8) % 2) ? 0.8f : 0.2f;
            s[3] = x < w * 3 / 4 ? 1.0f : 0.5f;
            for (int c = 0; c < 3; c++) p[c] = s[c] * s[3];
            p[3] = s[3];
        }
}

typedef struct {
    int w, h, hw, hh;
    float *straight;
    GLuint input;
    Target target;
    float *out; /* straight, w x h, y up */
} Frame;

static Frame frame(int w, int h, int hw, int hh) {
    Frame f = {w, h, hw, hh, calloc((size_t)w * h * 4, 4), 0, make_target(w, h), calloc((size_t)w * h * 4, 4)};
    float *texels = calloc((size_t)hw * hh * 4, 4);
    for (size_t i = 0; i < (size_t)hw * hh * 4; i++) texels[i] = 9.0f; /* padding the plugin must ignore */
    pattern(f.straight, texels, w, h, hw);
    f.input = make_texture(hw, hh, texels);
    free(texels);
    return f;
}

static int render(void *inst, Frame *f) {
    Texture t = {(uint32_t)f->w, (uint32_t)f->h, (uint32_t)f->hw, (uint32_t)f->hh, f->input};
    Texture *list[1] = {&t};
    Process p = {1, list, f->target.fbo};
    glBindFramebuffer(GL_FRAMEBUFFER, f->target.fbo);
    glViewport(0, 0, f->w, f->h);
    glClearColor(0, 0, 0, 0);
    glClear(GL_COLOR_BUFFER_BIT);
    uint32_t r = call(PROCESS_OPENGL, ptr(&p), inst).u;
    glReadPixels(0, 0, f->w, f->h, GL_RGBA, GL_FLOAT, f->out);
    for (int i = 0; i < f->w * f->h; i++) {
        float *o = &f->out[i * 4];
        if (o[3] > 0) for (int c = 0; c < 3; c++) o[c] /= o[3];
    }
    return r == 0;
}

static void *instantiate(int w, int h) {
    Viewport v = {0, 0, (GLuint)w, (GLuint)h};
    FFMixed r = call(INSTANTIATE_GL, ptr(&v), NULL);
    CHECK(r.p && (uintptr_t)r.p != FF_FAIL, "instantiate failed");
    double t = 0;
    call(SET_TIME, ptr(&t), r.p);
    return r.p;
}

static float max_diff(Frame *f) {
    float worst = 0;
    for (int i = 0; i < f->w * f->h * 4; i++) {
        float d = fabsf(f->out[i] - f->straight[i]);
        if (d > worst) worst = d;
    }
    return worst;
}

/* ---- Cases ------------------------------------------------------------------------- */

static void case_info(void) {
    PluginInfo *info = call(GET_INFO, u(0), NULL).p;
    CHECK(info && info->major == 2 && info->minor >= 1, "API version %u.%u", info ? info->major : 0, info ? info->minor : 0);
    CHECK(info && !strncmp(info->name, "VFX Preview", 11), "plugin name");
    CHECK(info && info->type == 0, "plugin type is effect");
    CHECK(call(GET_PLUGIN_CAPS, u(10), NULL).u == 1 && call(GET_PLUGIN_CAPS, u(11), NULL).u == 1, "one input");
    uint32_t n = call(GET_NUM_PARAMETERS, u(0), NULL).u;
    CHECK(n >= 10 && n < 64, "%u parameters", n);
    for (uint32_t i = 0; i < n; i++) {
        const char *name = call(GET_PARAMETER_NAME, u(i), NULL).p;
        CHECK(name && *name, "parameter %u has no name", i);
        GetRange r = {i, -99, -99};
        CHECK(call(GET_RANGE, ptr(&r), NULL).u == 0 && r.min < r.max, "%s range %g..%g", name, r.min, r.max);
        FFMixed d = call(GET_PARAMETER_DEFAULT, u(i), NULL);
        float dv;
        memcpy(&dv, &d.u, 4);
        CHECK(dv >= r.min && dv <= r.max, "%s default %g outside %g..%g", name, dv, r.min, r.max);
        if (call(GET_PARAMETER_TYPE, u(i), NULL).u == TYPE_OPTION) {
            uint32_t k = call(GET_NUM_PARAMETER_ELEMENTS, u(i), NULL).u;
            CHECK(k >= 2, "%s has %u options", name, k);
            for (uint32_t e = 0; e < k; e++) {
                GetElement ge = {i, e};
                const char *label = call(GET_PARAMETER_ELEMENT_NAME, ptr(&ge), NULL).p;
                CHECK(label && *label, "%s option %u has no label", name, e);
            }
        }
    }
    char group[64] = {0};
    GetString g = {(uint32_t)param("Brightness"), group, sizeof group - 1};
    CHECK(call(GET_PARAM_GROUP, ptr(&g), NULL).u == 0 && !strcmp(group, "Color"), "Brightness group is '%s'", group);
}

static void case_passthrough(void) {
    void *inst = instantiate(64, 48);
    Frame f = frame(64, 48, 64, 48);
    CHECK(render(inst, &f), "process failed");
    CHECK(max_diff(&f) < 1e-4, "defaults changed the image (max diff %g)", max_diff(&f));
    call(DEINSTANTIATE_GL, u(0), inst);
}

static void case_brightness(void) {
    void *inst = instantiate(64, 48);
    set(inst, "Brightness", 0.25f);
    FFMixed got = call(GET_PARAMETER, u((uint32_t)param("Brightness")), inst);
    float v; memcpy(&v, &got.u, 4);
    CHECK(fabsf(v - 0.25f) < 1e-6, "Brightness reads back %g", v);
    const char *shown = call(GET_PARAMETER_DISPLAY, u((uint32_t)param("Brightness")), inst).p;
    CHECK(shown && !strcmp(shown, "0.25"), "Brightness displays '%s'", shown ? shown : "(null)");
    Frame f = frame(64, 48, 64, 48);
    render(inst, &f);
    float want = px(f.straight, 64, 10, 5)[0] + 0.25f, gotr = px(f.out, 64, 10, 5)[0];
    CHECK(fabsf(gotr - want) < 1e-4, "red at (10,5) %g, expected %g", gotr, want);
    CHECK(px(f.out, 64, 10, 40)[1] > px(f.out, 64, 10, 5)[1], "image upside down");
    /* The half-transparent band: straight colour is brightened, alpha kept. */
    CHECK(fabsf(px(f.out, 64, 60, 20)[3] - 0.5f) < 1e-4, "alpha changed (%g)", px(f.out, 64, 60, 20)[3]);
    CHECK(fabsf(px(f.out, 64, 60, 20)[0] - (px(f.straight, 64, 60, 20)[0] + 0.25f)) < 1e-3, "half-transparent pixel wrong");
    call(DEINSTANTIATE_GL, u(0), inst);
}

static void case_padding(void) {
    /* A 50x40 image in a 64x64 allocation: the plugin must use only the image part. */
    void *inst = instantiate(50, 40);
    Frame f = frame(50, 40, 64, 64);
    CHECK(render(inst, &f), "process failed");
    CHECK(max_diff(&f) < 1e-4, "padded texture changed the image (max diff %g)", max_diff(&f));
    call(DEINSTANTIATE_GL, u(0), inst);
}

static void case_blur(void) {
    void *inst = instantiate(64, 48);
    set(inst, "Blur Radius", 4);
    Frame f = frame(64, 48, 64, 48);
    render(inst, &f);
    float edge = px(f.out, 64, 8, 4)[2];
    CHECK(edge > 0.25f && edge < 0.75f, "blue at the checker edge %g, expected a blend", edge);
    set(inst, "Blur Direction", 2); /* Vertical: no horizontal spread */
    render(inst, &f);
    float h = px(f.out, 64, 8, 4)[2];
    CHECK(fabsf(h - px(f.straight, 64, 8, 4)[2]) < 0.05f || fabsf(px(f.out, 64, 8, 4)[2] - px(f.out, 64, 9, 4)[2]) < 1e-3,
          "vertical blur spread horizontally");
    call(DEINSTANTIATE_GL, u(0), inst);
}

static void case_vignette(void) {
    void *inst = instantiate(64, 48);
    set(inst, "Vignette", 1);
    set(inst, "Vignette X", 0.25f);
    set(inst, "Vignette Y", 0.25f); /* bottom-left quadrant, y up */
    Frame f = frame(64, 48, 64, 48);
    render(inst, &f);
    /* Brightness relative to the input: near the centre ~1, the far corner darker. */
    float near = px(f.out, 64, 16, 12)[1] / fmaxf(px(f.straight, 64, 16, 12)[1], 1e-3f);
    float far = px(f.out, 64, 63, 47)[1] / fmaxf(px(f.straight, 64, 63, 47)[1], 1e-3f);
    CHECK(near > 0.95f, "vignette centre darkened (%g)", near);
    CHECK(far < 0.6f, "far corner not darkened (%g)", far);
    call(DEINSTANTIATE_GL, u(0), inst);
}

static void case_state(void) {
    /* FFGL hosts expect the plugin to leave GL state as it found it. */
    void *inst = instantiate(64, 48);
    set(inst, "Blur Radius", 2);
    Frame f = frame(64, 48, 64, 48);
    render(inst, &f);
    GLint v;
    glGetIntegerv(GL_CURRENT_PROGRAM, &v); CHECK(v == 0, "program %d left bound", v);
    glGetIntegerv(GL_TEXTURE_BINDING_2D, &v); CHECK(v == 0, "texture %d left bound", v);
    glGetIntegerv(GL_VERTEX_ARRAY_BINDING, &v); CHECK(v == 0, "vertex array %d left bound", v);
    glGetIntegerv(GL_ACTIVE_TEXTURE, &v); CHECK(v == GL_TEXTURE0, "active texture unit changed");
    glGetIntegerv(GL_FRAMEBUFFER_BINDING, &v); CHECK(v == (GLint)f.target.fbo, "host framebuffer unbound");
    GLint pack; glGetIntegerv(GL_PACK_ALIGNMENT, &pack); CHECK(pack == 4, "pack alignment %d", pack);
    CHECK(glGetError() == GL_NO_ERROR, "GL error after process");
    call(DEINSTANTIATE_GL, u(0), inst);
}

static double now(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec + t.tv_nsec * 1e-9; }

static void case_timing(void) {
    int sizes[][2] = {{1280, 720}, {1920, 1080}};
    for (int s = 0; s < 2; s++) {
        int w = sizes[s][0], h = sizes[s][1];
        void *inst = instantiate(w, h);
        set(inst, "Blur Radius", 8);
        set(inst, "Vignette", 1);
        Frame f = frame(w, h, w, h);
        render(inst, &f); /* warm up: chain built on first use */
        int n = 30;
        double t0 = now();
        for (int i = 0; i < n; i++) {
            double t = i / 60.0;
            call(SET_TIME, ptr(&t), inst);
            Texture tex = {(uint32_t)w, (uint32_t)h, (uint32_t)w, (uint32_t)h, f.input};
            Texture *list[1] = {&tex};
            Process p = {1, list, f.target.fbo};
            call(PROCESS_OPENGL, ptr(&p), inst);
        }
        glFinish();
        double ms = (now() - t0) * 1000 / n;
        printf("  %dx%d: %.1f ms per frame (blur 8 + vignette)\n", w, h, ms);
        call(DEINSTANTIATE_GL, u(0), inst);
    }
}

typedef struct { const char *name; void (*run)(void); } Case;
static Case cases[] = {
    {"info", case_info}, {"passthrough", case_passthrough}, {"brightness", case_brightness},
    {"padding", case_padding}, {"blur", case_blur}, {"vignette", case_vignette},
    {"state", case_state}, {"timing", case_timing},
};

int main(int argc, char **argv) {
    if (argc < 2) { fprintf(stderr, "usage: minihost <plugin binary> [case]\n"); return 2; }
    CGLPixelFormatAttribute attrs[] = {
        kCGLPFAOpenGLProfile, (CGLPixelFormatAttribute)kCGLOGLPVersion_GL4_Core,
        kCGLPFAAccelerated, kCGLPFAAllowOfflineRenderers, (CGLPixelFormatAttribute)0,
    };
    CGLPixelFormatObj pf; GLint npf; CGLContextObj ctx;
    if (CGLChoosePixelFormat(attrs, &pf, &npf) || !pf || CGLCreateContext(pf, NULL, &ctx)) {
        fprintf(stderr, "no OpenGL 4.1 core context\n"); return 2;
    }
    CGLSetCurrentContext(ctx);
    printf("GL %s on %s\n", glGetString(GL_VERSION), glGetString(GL_RENDERER));

    void *lib = dlopen(argv[1], RTLD_NOW | RTLD_LOCAL);
    if (!lib) { fprintf(stderr, "dlopen: %s\n", dlerror()); return 2; }
    plug = (PlugMain)dlsym(lib, "plugMain");
    if (!plug) { fprintf(stderr, "no plugMain export\n"); return 2; }
    if (call(INITIALISE_V2, u(0), NULL).u != 0) { fprintf(stderr, "initialise failed\n"); return 2; }

    const char *only = argc > 2 ? argv[2] : "all";
    int ran = 0;
    for (size_t i = 0; i < sizeof cases / sizeof cases[0]; i++) {
        if (strcmp(only, "all") && strcmp(only, cases[i].name)) continue;
        int before = failures;
        cases[i].run();
        printf("%-12s %s\n", cases[i].name, failures == before ? "ok" : "FAILED");
        ran++;
    }
    call(DEINITIALISE, u(0), NULL);
    if (!ran) { fprintf(stderr, "unknown case %s\n", only); return 2; }
    printf("%d case(s), %d failure(s)\n", ran, failures);
    return failures ? 1 : 0;
}
