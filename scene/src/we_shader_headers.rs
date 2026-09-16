use std::collections::HashSet;

#[derive(Debug, thiserror::Error)]
pub enum ShaderHeaderError {
    #[error("unknown #include \"{0}\" — not one of the known Wallpaper Engine shader headers (common.h, common_fragment.h, common_vertex.h, common_blending.h, common_perspective.h)")]
    UnknownInclude(String),
    #[error("\"{0}\" is #include-d more than once (directly or via a cycle) — refusing to expand it twice")]
    RepeatedInclude(String),
}

pub const PRELUDE_COMMON: &str = r#"
// Wallpaper Engine cross-API compatibility helpers — clean-room GLSL
// translation of docs.wallpaperengine.io/en/scene/shader/syntax.html.
// texSample2DLod is deliberately NOT here — see PRELUDE_VERTEX_ONLY /
// PRELUDE_FRAGMENT_ONLY: GLSL ES 1.00 only exposes unextended
// `texture2DLod` in vertex shaders; fragment shaders need
// `GL_EXT_shader_texture_lod` and its `texture2DLodEXT` entry point
// instead, so one shared body can't back both stages.
vec4 texSample2D(sampler2D tex, vec2 uv) { return texture2D(tex, uv); }

float frac(float x) { return fract(x); }
vec2  frac(vec2  x) { return fract(x); }
vec3  frac(vec3  x) { return fract(x); }
vec4  frac(vec4  x) { return fract(x); }

float saturate(float x) { return clamp(x, 0.0, 1.0); }
vec2  saturate(vec2  x) { return clamp(x, 0.0, 1.0); }
vec3  saturate(vec3  x) { return clamp(x, 0.0, 1.0); }
vec4  saturate(vec4  x) { return clamp(x, 0.0, 1.0); }

// GLSL's atan(y, x) two-argument form is the same operation as HLSL's atan2.
float atan2(float y, float x) { return atan(y, x); }

// HLSL's mul(v, M) treats v as a row vector (v * M) — confirmed against a
// real WE shader (the "tint" built-in Image Effect's vertex shader:
// `gl_Position = mul(vec4(a_Position, 1.0), g_ModelViewProjectionMatrix);`,
// found while investigating why a real Workshop item's effects weren't
// rendering — see docs/ROADMAP.md's M3 "effects" entry). GLSL's own
// `vector * matrix` operator computes exactly this row-vector product, so
// no manual transpose is needed. The reverse-order `mul(M, v)` overloads
// below aren't confirmed against any real WE shader (nothing seen uses
// them) but are kept for robustness in case some real content does.
vec2 mul(vec2 v, mat2 m) { return v * m; }
vec3 mul(vec3 v, mat3 m) { return v * m; }
vec4 mul(vec4 v, mat4 m) { return v * m; }
vec2 mul(mat2 m, vec2 v) { return m * v; }
vec3 mul(mat3 m, vec3 v) { return m * v; }
vec4 mul(mat4 m, vec4 v) { return m * v; }

// HLSL-style scalar-broadcast constructors; GLSL's vecN(scalar) constructor
// already does exactly this, so these just give WE shader source the name
// it expects.
vec2 CAST2(float x) { return vec2(x); }
vec3 CAST3(float x) { return vec3(x); }
vec4 CAST4(float x) { return vec4(x); }
mat3 CAST3X3(mat4 m) { return mat3(m); }
"#;

pub const PRELUDE_FRAGMENT_ONLY: &str = r#"
float ddx(float x) { return dFdx(x); }
vec2  ddx(vec2  x) { return dFdx(x); }
vec3  ddx(vec3  x) { return dFdx(x); }
float ddy(float x) { return dFdy(x); }
vec2  ddy(vec2  x) { return dFdy(x); }
vec3  ddy(vec3  x) { return dFdy(x); }

// Fragment-stage form of texSample2DLod — needs GL_EXT_shader_texture_lod
// (see PRELUDE_COMMON's doc comment); callers must emit that #extension
// line themselves.
vec4 texSample2DLod(sampler2D tex, vec2 uv, float lod) { return texture2DLodEXT(tex, uv, lod); }
"#;

pub const PRELUDE_VERTEX_ONLY: &str = r#"
vec4 texSample2DLod(sampler2D tex, vec2 uv, float lod) { return texture2DLod(tex, uv, lod); }
"#;

pub const HEADER_COMMON: &str = r#"
// `common.h` — plain math, no combo dependency, so this is the *real*
// Wallpaper Engine-compatible player's own verbatim source
// (github.com/laobamac/MirageWallpaper), not this project's earlier
// clean-room translation — see this constant's Rust doc comment
// (we_shader_headers.rs) for why this one was safe to swap outright
// while `common_fragment.h` (format-combo-dependent) wasn't.
#define M_PI 3.14159265359
#define M_PI_HALF 1.57079632679
#define M_PI_2 6.28318530718
#define SQRT_2 1.41421356237
#define SQRT_3 1.73205080756

vec3 hsv2rgb(vec3 c) {
    vec4 K = vec4(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
    // `fract`, not the `frac` HLSL-name alias — avoids a real prelude-vs-
    // header assembly-order dependency (`frac` is declared in
    // `PRELUDE_COMMON`, which a caller may inject either before or after
    // these headers; plain `fract` needs no such ordering at all).
    vec3 p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
    return c.z * mix(K.xxx, clamp(p - K.xxx, 0.0, 1.0), c.y);
}

vec3 rgb2hsv(vec3 RGB) {
    vec4 P = (RGB.g < RGB.b) ? vec4(RGB.bg, -1.0, 2.0 / 3.0) : vec4(RGB.gb, 0.0, -1.0 / 3.0);
    vec4 Q = (RGB.r < P.x) ? vec4(P.xyw, RGB.r) : vec4(RGB.r, P.yzx);
    float C = Q.x - min(Q.w, Q.y);
    float H = abs((Q.w - Q.y) / (6.0 * C + 1e-10) + Q.z);
    vec3 HCV = vec3(H, C, Q.x);
    float S = HCV.y / (HCV.z + 1e-10);
    return vec3(HCV.x, S, HCV.z);
}

vec2 rotateVec2(vec2 v, float r) {
    vec2 cs = vec2(cos(r), sin(r));
    return vec2(v.x * cs.x - v.y * cs.y, v.x * cs.y + v.y * cs.x);
}

// Real WE luma weights — independently confirmed twice against genuine
// content, not just this reference source: the real `godrays_downsample2.frag`
// inlines this exact same `dot(vec3(0.11, 0.59, 0.3), ...)` itself. This
// project's own prior clean-room version used the standard public BT.601
// weights (0.299/0.587/0.114) since WE's real constant wasn't published
// anywhere at the time — replaced now that it's confirmed real.
float greyscale(vec3 color) {
    return dot(color, vec3(0.11, 0.59, 0.3));
}
"#;

pub const HEADER_COMMON_FRAGMENT: &str = r#"
// Clean-room GLSL translation of docs.wallpaperengine.io/en/scene/shader/headers.html#common_fragment.h

// Standard DXT5nm/BC5-style normal decompression: X/Y stored in the
// alpha/green channels for better precision, Z reconstructed from the
// unit-length constraint. A well-known, public graphics technique — not
// WE-specific, but matches the documented "using alpha for enhanced
// precision" description.
vec3 DecompressNormal(vec4 packedNormal) {
    vec3 n;
    n.xy = packedNormal.ag * 2.0 - 1.0;
    n.z = sqrt(max(0.0, 1.0 - dot(n.xy, n.xy)));
    return n;
}

// Same decompression, plus a low-precision mask carried in an otherwise
// unused channel (the DXT5nm convention's red channel). The doc only says
// "with low-precision alpha masking" — which exact channel backs the mask
// isn't specified beyond that, so this is a best-effort inference from the
// standard DXT5nm-with-mask packing convention, not a confirmed fact.
vec4 DecompressNormalWithMask(vec4 packedNormal) {
    vec3 n = DecompressNormal(packedNormal);
    return vec4(n, packedNormal.r);
}

// "Handles single-channel 8-bit textures across multiple graphics APIs" —
// some APIs sample a single-channel texture into .r, others into .a; this
// takes the more common .r convention. Which APIs actually differ, and how
// WE's real implementation branches, isn't documented beyond the one-line
// purpose statement above.
float ConvertSampleR8(vec4 sampled) {
    return sampled.r;
}

// "Standardizes various texture formats to RGBA output with alpha
// support" — the actual per-format branching logic isn't documented at
// all, only its purpose, so this is an honest identity stub rather than a
// fabricated conversion table. Real content relying on non-RGBA source
// formats needing real standardization will not be handled correctly by
// this until WE's actual format list is confirmed.
vec4 ConvertTexture0Format(vec4 sampled) {
    return sampled;
}
"#;

pub const HEADER_COMMON_VERTEX: &str = r#"
// `common_vertex.h` — plain math, no combo dependency, so this is the
// real Wallpaper Engine-compatible player's own verbatim source
// (github.com/laobamac/MirageWallpaper), not this project's earlier
// clean-room translation. Real, notable difference from the earlier
// version: the world-space overload doesn't `normalize()` its basis
// vectors after transforming them (this project's own prior clean-room
// version did, a reasonable-looking but unconfirmed addition).
mat3 BuildTangentSpace(vec3 normal, vec4 signedTangent) {
    vec3 tangent = signedTangent.xyz;
    vec3 bitangent = cross(normal, tangent) * signedTangent.w;
    return mat3(tangent, bitangent, normal);
}

mat3 BuildTangentSpace(mat3 modelTransform, vec3 normal, vec4 signedTangent) {
    vec3 tangent = signedTangent.xyz;
    vec3 bitangent = cross(normal, tangent) * signedTangent.w;
    return mat3(mul(tangent, modelTransform), mul(bitangent, modelTransform), mul(normal, modelTransform));
}

// Real WE names this third overload `BuildTangentSpace` too (out-param
// overload of the one above); renamed here to avoid relying on naga's
// GLSL frontend correctly resolving two same-named functions that differ
// only by trailing `out` parameters, unconfirmed either way and not worth
// the risk for a function no real content calls at all yet.
void BuildTangentSpaceSplit(mat3 modelTransform, vec3 normal, vec4 signedTangent, out vec3 worldTangent, out vec3 worldBitangent) {
    vec3 tangent = signedTangent.xyz;
    vec3 bitangent = cross(normal, tangent) * signedTangent.w;
    worldTangent = mul(tangent, modelTransform);
    worldBitangent = mul(bitangent, modelTransform);
}
"#;

pub const HEADER_COMMON_BLENDING: &str = r#"
// A real Wallpaper Engine-compatible player's own `common_blending.h`
// (github.com/laobamac/MirageWallpaper) — see this constant's Rust doc
// comment (we_shader_headers.rs) for full provenance. Renamed to this
// project's own ApplyBlending parameter names; otherwise unmodified.
vec3 RGBToHSL(vec3 color) {
    vec3 hsl;
    float fmin = min(min(color.r, color.g), color.b);
    float fmax = max(max(color.r, color.g), color.b);
    float delta = fmax - fmin;
    hsl.z = (fmax + fmin) / 2.0;
    if (delta == 0.0) {
        hsl.x = 0.0;
        hsl.y = 0.0;
    } else {
        if (hsl.z < 0.5) {
            hsl.y = delta / (fmax + fmin);
        } else {
            hsl.y = delta / (2.0 - fmax - fmin);
        }
        float deltaR = (((fmax - color.r) / 6.0) + (delta / 2.0)) / delta;
        float deltaG = (((fmax - color.g) / 6.0) + (delta / 2.0)) / delta;
        float deltaB = (((fmax - color.b) / 6.0) + (delta / 2.0)) / delta;
        if (color.r == fmax) {
            hsl.x = deltaB - deltaG;
        } else if (color.g == fmax) {
            hsl.x = (1.0 / 3.0) + deltaR - deltaB;
        } else {
            hsl.x = (2.0 / 3.0) + deltaG - deltaR;
        }
        if (hsl.x < 0.0) {
            hsl.x += 1.0;
        } else if (hsl.x > 1.0) {
            hsl.x -= 1.0;
        }
    }
    return hsl;
}

float HueToRGB(float f1, float f2, float hue) {
    if (hue < 0.0) {
        hue += 1.0;
    } else if (hue > 1.0) {
        hue -= 1.0;
    }
    float res;
    if ((6.0 * hue) < 1.0) {
        res = f1 + (f2 - f1) * 6.0 * hue;
    } else if ((2.0 * hue) < 1.0) {
        res = f2;
    } else if ((3.0 * hue) < 2.0) {
        res = f1 + (f2 - f1) * ((2.0 / 3.0) - hue) * 6.0;
    } else {
        res = f1;
    }
    return res;
}

vec3 HSLToRGB(vec3 hsl) {
    vec3 rgb;
    if (hsl.y == 0.0) {
        rgb = vec3(hsl.z);
    } else {
        float f2;
        if (hsl.z < 0.5) {
            f2 = hsl.z * (1.0 + hsl.y);
        } else {
            f2 = (hsl.z + hsl.y) - (hsl.y * hsl.z);
        }
        float f1 = 2.0 * hsl.z - f2;
        rgb.r = HueToRGB(f1, f2, hsl.x + (1.0 / 3.0));
        rgb.g = HueToRGB(f1, f2, hsl.x);
        rgb.b = HueToRGB(f1, f2, hsl.x - (1.0 / 3.0));
    }
    return rgb;
}

float BlendLinearDodgef(float base, float blend) { return base + blend; }
float BlendLinearBurnf(float base, float blend) { return max(base + blend - 1.0, 0.0); }
float BlendLightenf(float base, float blend) { return max(blend, base); }
float BlendDarkenf(float base, float blend) { return min(blend, base); }
float BlendLinearLightf(float base, float blend) {
    return blend < 0.5 ? BlendLinearBurnf(base, 2.0 * blend) : BlendLinearDodgef(base, 2.0 * (blend - 0.5));
}
float BlendScreenf(float base, float blend) { return 1.0 - ((1.0 - base) * (1.0 - blend)); }
float BlendOverlayf(float base, float blend) {
    return base < 0.5 ? (2.0 * base * blend) : (1.0 - 2.0 * (1.0 - base) * (1.0 - blend));
}
float BlendSoftLightf(float base, float blend) {
    return (blend < 0.5)
        ? (2.0 * base * blend + base * base * (1.0 - 2.0 * blend))
        : (sqrt(base) * (2.0 * blend - 1.0) + 2.0 * base * (1.0 - blend));
}
float BlendColorDodgef(float base, float blend) { return (blend == 1.0) ? blend : min(base / (1.0 - blend), 1.0); }
float BlendColorBurnf(float base, float blend) { return (blend == 0.0) ? blend : max((1.0 - ((1.0 - base) / blend)), 0.0); }
float BlendVividLightf(float base, float blend) {
    return (blend < 0.5) ? BlendColorBurnf(base, 2.0 * blend) : BlendColorDodgef(base, 2.0 * (blend - 0.5));
}
float BlendPinLightf(float base, float blend) {
    return (blend < 0.5) ? BlendDarkenf(base, 2.0 * blend) : BlendLightenf(base, 2.0 * (blend - 0.5));
}
float BlendHardMixf(float base, float blend) { return (BlendVividLightf(base, blend) < 0.5) ? 0.0 : 1.0; }
float BlendReflectf(float base, float blend) { return (blend == 1.0) ? blend : min(base * base / (1.0 - blend), 1.0); }
vec3 BlendNormal(vec3 base, vec3 blend) { return blend; }
vec3 BlendLighten(vec3 base, vec3 blend) { return vec3(BlendLightenf(base.r, blend.r), BlendLightenf(base.g, blend.g), BlendLightenf(base.b, blend.b)); }
vec3 BlendDarken(vec3 base, vec3 blend) { return vec3(BlendDarkenf(base.r, blend.r), BlendDarkenf(base.g, blend.g), BlendDarkenf(base.b, blend.b)); }
vec3 BlendMultiply(vec3 base, vec3 blend) { return base * blend; }
vec3 BlendAverage(vec3 base, vec3 blend) { return (base + blend) / 2.0; }
vec3 BlendAdd(vec3 base, vec3 blend) { return min(base + blend, vec3(1.0)); }
vec3 BlendSubstract(vec3 base, vec3 blend) { return max(base + blend - vec3(1.0), vec3(0.0)); }
vec3 BlendDifference(vec3 base, vec3 blend) { return abs(base - blend); }
vec3 BlendNegation(vec3 base, vec3 blend) { return vec3(1.0) - abs(vec3(1.0) - base - blend); }
vec3 BlendExclusion(vec3 base, vec3 blend) { return base + blend - 2.0 * base * blend; }
vec3 BlendScreen(vec3 base, vec3 blend) { return vec3(BlendScreenf(base.r, blend.r), BlendScreenf(base.g, blend.g), BlendScreenf(base.b, blend.b)); }
vec3 BlendOverlay(vec3 base, vec3 blend) { return vec3(BlendOverlayf(base.r, blend.r), BlendOverlayf(base.g, blend.g), BlendOverlayf(base.b, blend.b)); }
vec3 BlendSoftLight(vec3 base, vec3 blend) { return vec3(BlendSoftLightf(base.r, blend.r), BlendSoftLightf(base.g, blend.g), BlendSoftLightf(base.b, blend.b)); }
vec3 BlendHardLight(vec3 base, vec3 blend) { return BlendOverlay(blend, base); }
vec3 BlendColorDodge(vec3 base, vec3 blend) { return vec3(BlendColorDodgef(base.r, blend.r), BlendColorDodgef(base.g, blend.g), BlendColorDodgef(base.b, blend.b)); }
vec3 BlendColorBurn(vec3 base, vec3 blend) { return vec3(BlendColorBurnf(base.r, blend.r), BlendColorBurnf(base.g, blend.g), BlendColorBurnf(base.b, blend.b)); }
vec3 BlendLinearLight(vec3 base, vec3 blend) { return vec3(BlendLinearLightf(base.r, blend.r), BlendLinearLightf(base.g, blend.g), BlendLinearLightf(base.b, blend.b)); }
vec3 BlendVividLight(vec3 base, vec3 blend) { return vec3(BlendVividLightf(base.r, blend.r), BlendVividLightf(base.g, blend.g), BlendVividLightf(base.b, blend.b)); }
vec3 BlendPinLight(vec3 base, vec3 blend) { return vec3(BlendPinLightf(base.r, blend.r), BlendPinLightf(base.g, blend.g), BlendPinLightf(base.b, blend.b)); }
vec3 BlendHardMix(vec3 base, vec3 blend) { return vec3(BlendHardMixf(base.r, blend.r), BlendHardMixf(base.g, blend.g), BlendHardMixf(base.b, blend.b)); }
vec3 BlendReflect(vec3 base, vec3 blend) { return vec3(BlendReflectf(base.r, blend.r), BlendReflectf(base.g, blend.g), BlendReflectf(base.b, blend.b)); }
vec3 BlendGlow(vec3 base, vec3 blend) { return BlendReflect(blend, base); }
vec3 BlendPhoenix(vec3 base, vec3 blend) { return min(base, blend) - max(base, blend) + vec3(1.0); }
vec3 BlendLinearDodge(vec3 base, vec3 blend) { return min(base + blend, vec3(1.0)); }
vec3 BlendLinearBurn(vec3 base, vec3 blend) { return max(base + blend - vec3(1.0), vec3(0.0)); }
vec3 BlendTint(vec3 base, vec3 blend) { return vec3(max(base.x, max(base.y, base.z))) * blend; }
// Each of these four hoists every `RGBToHSL` call into its own local
// before building the `vec3(...)` naga hands to `HSLToRGB` — same real
// nested-call-as-argument naga limitation `ApplyBlending` itself works
// around (see that function's comment).
vec3 BlendHue(vec3 base, vec3 blend) {
    vec3 baseHSL = RGBToHSL(base);
    vec3 blendHSL = RGBToHSL(blend);
    vec3 combined = vec3(blendHSL.r, baseHSL.g, baseHSL.b);
    return HSLToRGB(combined);
}
vec3 BlendSaturation(vec3 base, vec3 blend) {
    vec3 baseHSL = RGBToHSL(base);
    vec3 blendHSL = RGBToHSL(blend);
    vec3 combined = vec3(baseHSL.r, blendHSL.g, baseHSL.b);
    return HSLToRGB(combined);
}
vec3 BlendColor(vec3 base, vec3 blend) {
    vec3 baseHSL = RGBToHSL(base);
    vec3 blendHSL = RGBToHSL(blend);
    vec3 combined = vec3(blendHSL.r, blendHSL.g, baseHSL.b);
    return HSLToRGB(combined);
}
vec3 BlendLuminosity(vec3 base, vec3 blend) {
    vec3 baseHSL = RGBToHSL(base);
    vec3 blendHSL = RGBToHSL(blend);
    vec3 combined = vec3(baseHSL.r, baseHSL.g, blendHSL.b);
    return HSLToRGB(combined);
}

vec3 ApplyBlending(int mode, vec3 colorA, vec3 colorB, float blend) {
    // Every branch below hoists its `Blend*` call into a local variable
    // before passing it to `mix`/`min`/`max` — naga's GLSL frontend
    // otherwise fails real IR validation ("Expression not visited by the
    // appropriate statement") on a user-function call nested directly as
    // a builtin's argument; found the hard way when only the `mode`-0
    // fallback branch (the one real content actually exercises first)
    // tripped it, not a synthetic case that would've caught every branch
    // at once.
    vec3 blended;
#if BLENDMODE == 1
    blended = BlendDarken(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 2
    blended = BlendMultiply(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 3
    blended = BlendColorBurn(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 4
    blended = BlendSubstract(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 5
    return min(colorA, colorB);
#endif
#if BLENDMODE == 6
    blended = BlendLighten(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 7
    blended = BlendScreen(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 8
    blended = BlendColorDodge(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 9
    blended = BlendAdd(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 10
    return max(colorA, colorB);
#endif
#if BLENDMODE == 11
    blended = BlendOverlay(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 12
    blended = BlendSoftLight(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 13
    blended = BlendHardLight(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 14
    blended = BlendVividLight(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 15
    blended = BlendLinearLight(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 16
    blended = BlendPinLight(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 17
    blended = BlendHardMix(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 18
    blended = BlendDifference(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 19
    blended = BlendExclusion(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 20
    blended = BlendSubstract(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 21
    blended = BlendReflect(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 22
    blended = BlendGlow(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 23
    blended = BlendPhoenix(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 24
    blended = BlendAverage(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 25
    blended = BlendNegation(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 26
    blended = BlendHue(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 27
    blended = BlendSaturation(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 28
    blended = BlendColor(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 29
    blended = BlendLuminosity(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 30
    blended = BlendTint(colorA, colorB);
    return mix(colorA, blended, blend);
#endif
#if BLENDMODE == 31
    return colorA + colorB * blend;
#endif
#if BLENDMODE == 32
    blended = colorA + colorA * colorB;
    return mix(colorA, blended, blend);
#endif
    // `BlendNormal(A, B) == B` — inlined directly rather than calling the
    // wrapper: naga's GLSL frontend fails real IR validation
    // ("Expression not visited by the appropriate statement") specifically
    // on a call to this one trivial single-statement wrapper here, for
    // reasons not fully understood (every other real `Blend*` wrapper
    // above, called the exact same way, compiles fine) — sidestepped
    // rather than chased further, since the equation itself needs no
    // wrapper at all to express correctly.
    blended = colorB;
    return mix(colorA, blended, blend);
}
"#;

pub const HEADER_COMMON_PERSPECTIVE: &str = r#"
// A real Wallpaper Engine-compatible player's own `common_perspective.h`
// (github.com/laobamac/MirageWallpaper), minus its `#if HLSL` inverse()
// polyfill — see this constant's Rust doc comment (we_shader_headers.rs)
// for full provenance.
mat3 squareToQuad(vec2 p0, vec2 p1, vec2 p2, vec2 p3) {
    mat3 m = mat3(1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0);
    float dx0 = p0.x;
    float dy0 = p0.y;
    float dx1 = p1.x;
    float dy1 = p1.y;

    float dx2 = p3.x;
    float dy2 = p3.y;
    float dx3 = p2.x;
    float dy3 = p2.y;

    float diffx1 = dx1 - dx3;
    float diffy1 = dy1 - dy3;
    float diffx2 = dx2 - dx3;
    float diffy2 = dy2 - dy3;

    float det = diffx1 * diffy2 - diffx2 * diffy1;
    float sumx = dx0 - dx1 + dx3 - dx2;
    float sumy = dy0 - dy1 + dy3 - dy2;

    if (det == 0.0 || (sumx == 0.0 && sumy == 0.0)) {
        m[0][0] = dx1 - dx0;
        m[0][1] = dy1 - dy0;
        m[0][2] = 0.0;
        m[1][0] = dx3 - dx1;
        m[1][1] = dy3 - dy1;
        m[1][2] = 0.0;
        m[2][0] = dx0;
        m[2][1] = dy0;
        m[2][2] = 1.0;
        return m;
    } else {
        float ovdet = 1.0 / det;
        float g = (sumx * diffy2 - diffx2 * sumy) * ovdet;
        float h = (diffx1 * sumy - sumx * diffy1) * ovdet;

        m[0][0] = dx1 - dx0 + g * dx1;
        m[0][1] = dy1 - dy0 + g * dy1;
        m[0][2] = g;
        m[1][0] = dx2 - dx0 + h * dx2;
        m[1][1] = dy2 - dy0 + h * dy2;
        m[1][2] = h;
        m[2][0] = dx0;
        m[2][1] = dy0;
        m[2][2] = 1.0;
        return m;
    }
}
"#;

pub const HEADER_COMMON_BLUR: &str = r#"
// A real Wallpaper Engine-compatible player's own `common_blur.h`
// (github.com/laobamac/MirageWallpaper) — see this constant's Rust doc
// comment (we_shader_headers.rs) for full provenance. Verbatim.
vec3 blur13(vec2 u, vec2 d) {
    vec2 o1 = CAST2(1.4091998770852122) * d;
    vec2 o2 = CAST2(3.2979348079914822) * d;
    vec2 o3 = CAST2(5.2062900776825969) * d;
    return texSample2D(g_Texture0, u).rgb * 0.1976406528809576
        + texSample2D(g_Texture0, u + o1).rgb * 0.2959855056006557
        + texSample2D(g_Texture0, u - o1).rgb * 0.2959855056006557
        + texSample2D(g_Texture0, u + o2).rgb * 0.0935333619980593
        + texSample2D(g_Texture0, u - o2).rgb * 0.0935333619980593
        + texSample2D(g_Texture0, u + o3).rgb * 0.0116608059608062
        + texSample2D(g_Texture0, u - o3).rgb * 0.0116608059608062;
}
vec3 blur7(vec2 u, vec2 d) {
    vec2 o1 = CAST2(2.3515644035337887) * d;
    vec2 o2 = CAST2(0.469433779698372) * d;
    vec2 o3 = CAST2(1.4091998770852121) * d;
    vec2 o4 = CAST2(3.0) * d;
    return texSample2D(g_Texture0, u + o1).rgb * 0.2028175528299753
        + texSample2D(g_Texture0, u + o2).rgb * 0.4044856614512112
        + texSample2D(g_Texture0, u - o3).rgb * 0.3213933537319605
        + texSample2D(g_Texture0, u - o4).rgb * 0.0713034319868530;
}
vec3 blur3(vec2 u, vec2 d) {
    return texSample2D(g_Texture0, u + d).rgb * 0.25
        + texSample2D(g_Texture0, u).rgb * 0.5
        + texSample2D(g_Texture0, u - d).rgb * 0.25;
}
vec4 blur13a(vec2 u, vec2 d) {
    vec2 o1 = CAST2(1.4091998770852122) * d;
    vec2 o2 = CAST2(3.2979348079914822) * d;
    vec2 o3 = CAST2(5.2062900776825969) * d;
    return texSample2D(g_Texture0, u) * 0.1976406528809576
        + texSample2D(g_Texture0, u + o1) * 0.2959855056006557
        + texSample2D(g_Texture0, u - o1) * 0.2959855056006557
        + texSample2D(g_Texture0, u + o2) * 0.0935333619980593
        + texSample2D(g_Texture0, u - o2) * 0.0935333619980593
        + texSample2D(g_Texture0, u + o3) * 0.0116608059608062
        + texSample2D(g_Texture0, u - o3) * 0.0116608059608062;
}
vec4 blur7a(vec2 u, vec2 d) {
    vec2 o1 = CAST2(2.3515644035337887) * d;
    vec2 o2 = CAST2(0.469433779698372) * d;
    vec2 o3 = CAST2(1.4091998770852121) * d;
    vec2 o4 = CAST2(3.0) * d;
    return texSample2D(g_Texture0, u + o1) * 0.2028175528299753
        + texSample2D(g_Texture0, u + o2) * 0.4044856614512112
        + texSample2D(g_Texture0, u - o3) * 0.3213933537319605
        + texSample2D(g_Texture0, u - o4) * 0.0713034319868530;
}
vec4 blur3a(vec2 u, vec2 d) {
    return texSample2D(g_Texture0, u + d) * 0.25
        + texSample2D(g_Texture0, u) * 0.5
        + texSample2D(g_Texture0, u - d) * 0.25;
}
vec2 blurRotateVec2(vec2 v, float r) {
    vec2 cs = vec2(cos(r), sin(r));
    return vec2(v.x * cs.x - v.y * cs.y, v.x * cs.y + v.y * cs.x);
}
vec4 blurRadial13a(vec2 u, vec2 center, float amt) {
    vec2 delta = u - center;
    amt = amt * 0.025;
    float o1 = 1.4091998770852122 * amt;
    float o2 = 3.2979348079914822 * amt;
    float o3 = 5.2062900776825969 * amt;
    vec2 r1 = blurRotateVec2(delta, o1) - delta;
    vec2 r2 = blurRotateVec2(delta, o2) - delta;
    vec2 r3 = blurRotateVec2(delta, o3) - delta;
    return texSample2D(g_Texture0, u) * 0.1976406528809576
        + texSample2D(g_Texture0, center + r1 + delta) * 0.2959855056006557
        + texSample2D(g_Texture0, center - r1 + delta) * 0.2959855056006557
        + texSample2D(g_Texture0, center + r2 + delta) * 0.0935333619980593
        + texSample2D(g_Texture0, center - r2 + delta) * 0.0935333619980593
        + texSample2D(g_Texture0, center + r3 + delta) * 0.0116608059608062
        + texSample2D(g_Texture0, center - r3 + delta) * 0.0116608059608062;
}
vec4 blurRadial7a(vec2 u, vec2 center, float amt) {
    vec2 delta = u - center;
    amt = amt * 0.025;
    float o1 = 2.3515644035337887 * amt;
    float o2 = 0.469433779698372 * amt;
    float o3 = 1.4091998770852121 * amt;
    float o4 = 3.0 * amt;
    vec2 r1 = blurRotateVec2(delta, o1) - delta;
    vec2 r2 = blurRotateVec2(delta, o2) - delta;
    vec2 r3 = blurRotateVec2(delta, -o3) - delta;
    vec2 r4 = blurRotateVec2(delta, -o4) - delta;
    return texSample2D(g_Texture0, center + r1 + delta) * 0.2028175528299753
        + texSample2D(g_Texture0, center + r2 + delta) * 0.4044856614512112
        + texSample2D(g_Texture0, center + r3 + delta) * 0.3213933537319605
        + texSample2D(g_Texture0, center + r4 + delta) * 0.0713034319868530;
}
vec4 blurRadial3a(vec2 u, vec2 center, float amt) {
    vec2 delta = u - center;
    amt = amt * 0.025;
    float o1 = amt;
    vec2 r1 = blurRotateVec2(delta, o1) - delta;
    return texSample2D(g_Texture0, center + delta) * 0.5
        + texSample2D(g_Texture0, center + r1 + delta) * 0.25
        + texSample2D(g_Texture0, center - r1 + delta) * 0.25;
}
"#;

pub const HEADER_COMMON_COMPOSITE: &str = r#"
// A real Wallpaper Engine-compatible player's own `common_composite.h`
// (github.com/laobamac/MirageWallpaper) — see this constant's Rust doc
// comment (we_shader_headers.rs) for full provenance. Verbatim.
uniform float g_CompositeAlpha; // {"material":"compositealpha","label":"ui_editor_properties_alpha","default":1,"range":[0.0, 2.0]}
uniform vec2 g_CompositeOffset; // {"material":"compositeoffset","label":"ui_editor_properties_offset","default":"0 0","linked":true,"range":[-10.0, 10.0]}
uniform vec3 g_CompositeColor; // {"material":"compositecolor","label":"ui_editor_properties_color","default":"1 1 1","type":"color"}

vec2 ApplyCompositeOffset(vec2 texCoords, vec2 textureResolution) {
#if COMPOSITE != 0
    return texCoords + g_CompositeOffset / textureResolution;
#else
    return texCoords;
#endif
}

vec4 ApplyComposite(vec4 original, vec4 effect) {
#if COMPOSITEMONO == 1
    effect.rgb = CAST3(greyscale(effect.rgb));
#endif

    effect.rgb *= g_CompositeColor;

#if COMPOSITE == 0
    return effect;
#endif

#if COMPOSITE == 1
    effect.rgb = ApplyBlending(BLENDMODE, original.rgb, effect.rgb, effect.a * g_CompositeAlpha);
    effect.a = max(effect.a * saturate(g_CompositeAlpha), original.a);
#endif

#if COMPOSITE == 2
    effect.a *= saturate(g_CompositeAlpha);
    effect = mix(effect, original, original.a);
#endif

#if COMPOSITE == 3
    effect.a *= saturate(g_CompositeAlpha);
    effect.a *= 1.0 - original.a;
#endif

    return effect;
}
"#;

pub const HEADER_COMMON_FOG: &str = r#"
// A real Wallpaper Engine-compatible player's own `common_fog.h`
// (github.com/laobamac/MirageWallpaper) — see this constant's Rust doc
// comment (we_shader_headers.rs) for full provenance. Verbatim.
#if FOG_DIST
uniform vec3 g_FogDistanceColor;
uniform vec4 g_FogDistanceParams;
#endif

#if FOG_HEIGHT
uniform vec3 g_FogHeightColor;
uniform vec4 g_FogHeightParams;
#endif

vec2 CalculateFogPixelState(float viewDirLength, float worldPosHeight) {
    vec2 result = CAST2(0.0);
#if FOG_DIST
    result.x = (viewDirLength - g_FogDistanceParams.x) / g_FogDistanceParams.y;
#endif
#if FOG_HEIGHT
    result.y = (worldPosHeight - g_FogHeightParams.x) / g_FogHeightParams.y;
#endif
    return result;
}

vec3 ApplyFog(vec3 color, vec2 fogPixelState) {
#if FOG_HEIGHT
    float fogHeight = saturate(fogPixelState.y);
    color.rgb = mix(color.rgb, g_FogHeightColor, g_FogHeightParams.z + g_FogHeightParams.w * fogHeight * fogHeight);
#endif
#if FOG_DIST
    float fogDistance = saturate(fogPixelState.x);
    color.rgb = mix(color.rgb, g_FogDistanceColor, g_FogDistanceParams.z + g_FogDistanceParams.w * fogDistance * fogDistance);
#endif
    return color;
}

float ApplyFogAlpha(float alpha, vec2 fogPixelState) {
#if FOG_DIST
    float fogDistance = saturate(fogPixelState.x);
    fogDistance = g_FogDistanceParams.z + g_FogDistanceParams.w * fogDistance * fogDistance;
#else
    float fogDistance = 0.0;
#endif
#if FOG_HEIGHT
    float fogHeight = saturate(fogPixelState.y);
    fogHeight = g_FogHeightParams.z + g_FogHeightParams.w * fogHeight * fogHeight;
#else
    float fogHeight = 0.0;
#endif
    float fogFactor = saturate(max(fogDistance, fogHeight));
    return alpha * (1.0 - (fogFactor * fogFactor));
}
"#;

pub const HEADER_COMMON_FOLIAGE: &str = r#"
// A real Wallpaper Engine-compatible player's own `common_foliage.h`
// (github.com/laobamac/MirageWallpaper) — see this constant's Rust doc
// comment (we_shader_headers.rs) for full provenance. Verbatim.
float CalcLeavesUVWeight(vec2 uvs, vec2 uvBounds) {
#if LEAVESUVMODE == 1
    return saturate((1.0 - uvs.y - uvBounds.x) * uvBounds.y);
#elif LEAVESUVMODE == 2
    return saturate((uvs.y - uvBounds.x) * uvBounds.y);
#elif LEAVESUVMODE == 3
    return saturate((uvs.x - uvBounds.x) * uvBounds.y);
#elif LEAVESUVMODE == 4
    return saturate((1.0 - uvs.x - uvBounds.x) * uvBounds.y);
#endif
    return 1.0;
}

vec3 CalcFoliageAnimation(
    vec3 worldPos, vec3 localPos, vec2 uvs, float direction, float time,
    float speedLeaves, float speedBase, float strengthLeaves, float strengthBase,
    float phase, float scale, float cutoff, float treeHeight, float treeRadius, vec2 uvBounds
) {
    vec3 foliageOffsetForward = vec3(cos(direction), 0.0, sin(direction));
    vec3 foliageOffsetUp = vec3(0.0, 1.0, 0.0);

    vec4 fastSines = sin(phase + speedLeaves * time * vec4(1.71717171, -1.56161616, -1.9333, 1.041666666) + worldPos.xzzy * scale * 3.333);
    vec4 slowSines = sin(phase + speedBase * time * vec4(0.53333, -0.019841, -0.13888889, 0.0024801587) + worldPos.xyyx * scale);
    fastSines = smoothstep(CAST4(cutoff) + fastSines * 0.1, CAST4(1.0 - cutoff) - fastSines.zwyx * 0.1, fastSines * CAST4(0.5) + CAST4(0.5)) * CAST4(2.0) - CAST4(1.0);
    float cutoffBase = cutoff * 0.6666;
    slowSines = smoothstep(CAST4(cutoffBase) + slowSines * 0.1, CAST4(1.0 - cutoffBase) - slowSines.zwyx * 0.1, slowSines * CAST4(0.5) + CAST4(0.5)) * CAST4(2.0) - CAST4(1.0);

    float leafMask = strengthLeaves * smoothstep(-1.2, -0.3, sin(dot(worldPos.xyz, foliageOffsetForward) + speedBase * time));
    float leafDistance = dot(localPos.xz, localPos.xz);
    float baseMask = smoothstep(0.0, treeHeight, localPos.y);

    vec2 blendParamsA = vec2(treeRadius * treeRadius, treeRadius);
    vec2 blendParamsB = vec2(treeRadius, treeRadius * treeRadius);
    vec2 blendParams = mix(blendParamsA, blendParamsB, step(1.0, treeRadius));
    leafMask *= mix(smoothstep(blendParams.x, blendParams.y, leafDistance), baseMask, baseMask) * CalcLeavesUVWeight(uvs, uvBounds);
    baseMask *= strengthBase;

    vec4 strengthMask = vec4(leafMask, leafMask, baseMask, baseMask);
    return dot(strengthMask, vec4(fastSines.xy, slowSines.xy)) * foliageOffsetForward
        + dot(strengthMask, vec4(fastSines.zw, slowSines.zw)) * foliageOffsetUp;
}
"#;

pub const HEADER_COMMON_PBR: &str = r#"
// A real Wallpaper Engine-compatible player's own `common_pbr.h`
// (github.com/laobamac/MirageWallpaper) — see this constant's Rust doc
// comment (we_shader_headers.rs) for full provenance, and its stated
// "unverified" caveat. **Not fully verbatim, unlike every other header
// here**: `ComputePBRLight`'s real `#ifdef GRADIENT_SAMPLER`/`#if
// RIMLIGHTING` branches are dropped, keeping only the plain (no gradient
// remap, no rim light) path — the `#if RIMLIGHTING` form would hit the
// same "undefined macro" naga preprocessor error `BLENDMODE` did before
// this project defined it, and `GRADIENT_SAMPLER` is a real sampler
// binding, not a simple combo value, needing actual texture-binding
// infrastructure this project has no real content to justify building
// yet. `PointSegmentDelta` (present, unused by anything else in this
// header, kept for completeness) is verbatim.
vec3 FresnelSchlick(float lightTheta, vec3 baseReflectance) {
    return baseReflectance + (1.0 - baseReflectance) * pow(max(1.0 - lightTheta, 0.001), 5.0);
}

vec3 PointSegmentDelta(vec3 pos, vec3 segmentA, vec3 segmentB) {
    vec3 delta = segmentB - segmentA;
    float v = dot(delta, delta);
    if (v == 0.0) {
        return segmentA - pos;
    }
    return segmentA + saturate(dot(pos - segmentA, segmentB - segmentA) / v) * (segmentB - segmentA) - pos;
}

float Distribution_GGX(vec3 N, vec3 H, float roughness) {
    float rSqr = roughness * roughness;
    float rSqr2 = rSqr * rSqr;
    float NH = max(dot(N, H), 0.0);
    float denominator = (NH * NH * (rSqr2 - 1.0) + 1.0);
    return rSqr2 / (M_PI * denominator * denominator);
}

float Schlick_GGX(float NV, float roughness) {
    float roughnessBase = roughness + 1.0;
    float roughnessScaled = (roughnessBase * roughnessBase) / 8.0;
    return NV / (NV * (1.0 - roughnessScaled) + roughnessScaled);
}

float GeoSmith(vec3 N, vec3 V, vec3 L, float roughness) {
    return Schlick_GGX(max(dot(N, V), 0.001), roughness) * Schlick_GGX(max(dot(N, L), 0.001), roughness);
}

// L = worldToLightVector, N = normalVector, V = worldToViewVector
vec3 ComputePBRLight(vec3 N, vec3 L, vec3 V, vec3 albedo, vec3 lightColor, vec3 baseReflectance, float roughness, float metallic) {
    float distance = length(L);
    L = L / distance;
    vec3 H = normalize(V + L);

    float NDF = Distribution_GGX(N, H, roughness);
    float G = GeoSmith(N, V, L, roughness);
    vec3 F = FresnelSchlick(max(dot(H, V), 0.0), baseReflectance);
    vec3 numerator = NDF * G * F;

    float dNL = dot(N, L);
    float NL = max(dNL, 0.0);
    float denominator = 4.0 * max(dot(N, V), 0.0) * NL;
    vec3 specular = numerator / max(denominator, 0.001);

    vec3 diffuse = (1.0 - metallic) * (vec3(1.0) - F);
    vec3 radiance = lightColor.xyz / (distance * distance);
    return (diffuse * albedo / M_PI + specular) * radiance * NL;
}

vec3 CombineLighting(vec3 light, vec3 ambient) {
#if HDR
    float lightLen = length(light);
    float overbright = (saturate(lightLen - 2.0) * 0.5) / max(0.01, lightLen);
    return saturate(ambient + light) + (light * overbright);
#else
    return ambient + light;
#endif
}
"#;

pub const HEADER_COMMON_PBR_2: &str = r#"
// A real Wallpaper Engine-compatible player's own `common_pbr_2.h`
// (github.com/laobamac/MirageWallpaper) — see this constant's Rust doc
// comment (we_shader_headers.rs) for full provenance, and its stated
// "unverified" caveat. **Not fully verbatim**: the real file's entire
// `#ifdef SHADOW_ATLAS_SAMPLER` block (`PerformShadowMapping`/
// `PerformPointShadowMapping`) and its point/cascade projection helpers
// are dropped — they call `texSample2DCompare`, a comparison-sampler
// builtin this project's shader compiler has no equivalent of at all, so
// including them verbatim would just be dead text no real GLSL toolchain
// here could ever validate even syntactically. Kept: the base GGX
// functions (identical to `HEADER_COMMON_PBR`'s own — real WE ships them
// duplicated across both files, not shared) plus the one real shadow-
// *aware* lighting function that doesn't need the atlas sampler,
// `ComputePBRLightShadowInfinite` (its own `#ifdef GRADIENT_SAMPLER`/`#if
// RIMLIGHTING` branches dropped for the same reason as `HEADER_COMMON_PBR`'s).
vec3 FresnelSchlick(float lightTheta, vec3 baseReflectance) {
    return baseReflectance + (1.0 - baseReflectance) * pow(max(1.0 - lightTheta, 0.001), 5.0);
}

float Distribution_GGX(vec3 N, vec3 H, float roughness) {
    float rSqr = roughness * roughness;
    float rSqr2 = rSqr * rSqr;
    float NH = max(dot(N, H), 0.0);
    float denominator = (NH * NH * (rSqr2 - 1.0) + 1.0);
    return rSqr2 / (M_PI * denominator * denominator);
}

float Schlick_GGX(float NV, float roughness) {
    float roughnessBase = roughness + 1.0;
    float roughnessScaled = (roughnessBase * roughnessBase) / 8.0;
    return NV / (NV * (1.0 - roughnessScaled) + roughnessScaled);
}

float GeoSmith(vec3 N, vec3 V, vec3 L, float roughness) {
    return Schlick_GGX(max(dot(N, V), 0.001), roughness) * Schlick_GGX(max(dot(N, L), 0.001), roughness);
}

vec3 CalculateProjectedCoords(vec3 worldPos, mat4 shadowViewProjection) {
    vec4 proj = mul(vec4(worldPos, 1.0), shadowViewProjection);
    proj.xyz /= proj.w;
    proj.xy = proj.xy * vec2(0.5, -0.5) + CAST2(0.5);
    proj.y = mix(proj.y, 2.0, step(proj.w, 0.0));
    return proj.xyz;
}

// L = worldToLightVector, N = normalVector, V = worldToViewVector
vec3 ComputePBRLightShadowInfinite(
    vec3 N, vec3 L, vec3 V, vec3 albedo, vec3 lightColor,
    vec3 specularTint, vec3 baseReflectance, float roughness, float metallic, float shadowFactor
) {
    vec3 H = normalize(V + L);
    float NDF = shadowFactor * Distribution_GGX(N, H, roughness);
    float G = GeoSmith(N, V, L, roughness);
    vec3 F = FresnelSchlick(max(dot(H, V), 0.0), baseReflectance);
    vec3 numerator = NDF * G * F;

    float dNL = dot(N, L);
    float NL = max(dNL * shadowFactor, 0.0);
    float denominator = 4.0 * max(dot(N, V), 0.0) * NL;
    vec3 specular = numerator / max(denominator, 0.001);

    vec3 diffuse = (1.0 - metallic) * (vec3(1.0) - F);
    return (diffuse * albedo / M_PI + specular * specularTint) * lightColor * NL;
}

vec3 CombineLighting(vec3 light, vec3 ambient) {
#if HDR
    float lightLen = length(light);
    float overbright = (saturate(lightLen - 2.0) * 0.5) / max(0.01, lightLen);
    return saturate(ambient + light) + (light * overbright);
#else
    return ambient + light;
#endif
}

vec3 CombineLighting(vec3 light, vec3 baseAmbient, vec3 ambient) {
#if HDR
    float lightLen = length(light);
    float overbright = (saturate(lightLen - 2.0) * 0.5) / max(0.01, lightLen);
    return max(baseAmbient, saturate(ambient + light)) + (light * overbright);
#else
    return max(baseAmbient, ambient + light);
#endif
}
"#;

pub fn resolve_includes(source: &str) -> Result<String, ShaderHeaderError> {
    let mut seen = HashSet::new();
    let mut out = String::with_capacity(source.len());
    for line in source.lines() {
        if let Some(name) = parse_include(line) {
            if !seen.insert(name.to_string()) {
                return Err(ShaderHeaderError::RepeatedInclude(name.to_string()));
            }
            out.push_str(
                known_header(name)
                    .ok_or_else(|| ShaderHeaderError::UnknownInclude(name.to_string()))?,
            );
            out.push('\n');
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    Ok(out)
}

fn parse_include(line: &str) -> Option<&str> {
    let rest = line.trim().strip_prefix("#include")?;
    let rest = rest.trim();
    let rest = rest.strip_prefix('"')?;
    rest.split('"').next()
}

fn known_header(name: &str) -> Option<&'static str> {
    match name {
        "common.h" => Some(HEADER_COMMON),
        "common_fragment.h" => Some(HEADER_COMMON_FRAGMENT),
        "common_vertex.h" => Some(HEADER_COMMON_VERTEX),
        "common_blending.h" => Some(HEADER_COMMON_BLENDING),
        "common_perspective.h" => Some(HEADER_COMMON_PERSPECTIVE),
        "common_blur.h" => Some(HEADER_COMMON_BLUR),
        "common_composite.h" => Some(HEADER_COMMON_COMPOSITE),
        "common_fog.h" => Some(HEADER_COMMON_FOG),
        "common_foliage.h" => Some(HEADER_COMMON_FOLIAGE),
        "common_pbr.h" => Some(HEADER_COMMON_PBR),
        "common_pbr_2.h" => Some(HEADER_COMMON_PBR_2),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_a_known_include() {
        let src = "#include \"common.h\"\nvoid main() {}\n";
        let out = resolve_includes(src).unwrap();
        assert!(out.contains("float greyscale"));
        assert!(out.contains("void main() {}"));
    }

    #[test]
    fn rejects_unknown_include_by_name() {
        let err = resolve_includes("#include \"nope.h\"\n").unwrap_err();
        match err {
            ShaderHeaderError::UnknownInclude(name) => assert_eq!(name, "nope.h"),
            other => panic!("expected UnknownInclude, got: {other}"),
        }
    }

    #[test]
    fn rejects_the_same_include_twice() {
        let src = "#include \"common.h\"\n#include \"common.h\"\n";
        let err = resolve_includes(src).unwrap_err();
        match err {
            ShaderHeaderError::RepeatedInclude(name) => assert_eq!(name, "common.h"),
            other => panic!("expected RepeatedInclude, got: {other}"),
        }
    }

    #[test]
    fn leaves_non_include_lines_untouched() {
        let src = "uniform sampler2D g_Texture0;\n#include \"common_fragment.h\"\nvoid main() {}\n";
        let out = resolve_includes(src).unwrap();
        assert!(out.starts_with("uniform sampler2D g_Texture0;\n"));
        assert!(out.contains("ConvertTexture0Format"));
    }
}
