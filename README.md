<h1 align="center">wallpaper-engine-rosetta</h1>

<div align="center">
Renders Wallpaper Engine scenes outside Wallpaper Engine — a Rust/wgpu library any app can link
  <br />
  <br />
  <a href="https://github.com/Nytuo/wallpaper-engine-rosetta/issues/new?labels=bug&title=bug%3A+">Report a Bug</a>
  ·
  <a href="https://github.com/Nytuo/wallpaper-engine-rosetta/issues/new?labels=enhancement&title=feat%3A+">Request a Feature</a>
  ·
  <a href="https://github.com/Nytuo/wallpaper-engine-rosetta/discussions">Ask a Question</a>

</div>

<div align="center">
<br />

[![Project license](https://img.shields.io/github/license/Nytuo/wallpaper-engine-rosetta.svg?style=flat-square)](LICENSE)

[![code with love by Nytuo](https://img.shields.io/badge/%3C%2F%3E%20with%20%E2%99%A5%20by-Nytuo-ff1414.svg?style=flat-square)](https://github.com/Nytuo)

</div>

<details open="open">
<summary>Table of Contents</summary>

- [About](#about)
- [What It Can Do](#what-it-can-do)
- [Technologies](#technologies)
- [Status: known gaps](#status-known-gaps)
- [Using This From Another App](#using-this-from-another-app)
- [Testing](#testing)
- [Authors \& contributors](#authors--contributors)
- [License](#license)

</details>

---

## About

Wallpaper Engine's Workshop holds a huge library of animated desktop
wallpapers, in a scene format never documented, playable only inside
Wallpaper Engine itself, on Windows. wallpaper-engine-rosetta reads that
format — the packed `.pkg`/`.tex` container formats, the `scene.json` object
graph, and Wallpaper Engine's own shader dialect — and renders it for real, on
a GPU, through [wgpu](https://wgpu.rs).It links into any app that
can call a C ABI.

> **Beta.** The scene format is undocumented, so everything here was worked
> out from real downloaded Workshop items rather than from a spec. A scene can
> look wrong, be missing an effect, or fail to load — see
> [Status: known gaps](#status-known-gaps) for exactly what that covers today.

## What It Can Do

- **Read Wallpaper Engine's real, undocumented formats** — the `.pkg` archive
  a Scene Workshop item ships as and the `.tex` texture container its
  materials use, reverse engineered and verified against real downloaded
  items (cross-checked against
  [RePKG](https://github.com/notscuffed/repkg) where useful).

- **Translate a real `scene.json`** into layered image objects with their
  size, scale and placement, resolving each one's material -> texture chain.

- **Compile Wallpaper Engine's own shader dialect** to real GPU shaders — a
  clean-room reimplementation of its `#include` headers, a GLSL-dialect
  transpiler, and reflection-driven pipeline construction — and run the
  object effect chain it drives, including multi-pass effects such as god
  rays, through a real named-render-target compositor.

- **Simulate real particle systems** — emitters, initializers and operators —
  and render them as additively-blended instanced billboards.

- **Animate what Wallpaper Engine animates**: time-varying effects
  (`waterwaves`, `shake`, ...) and mouse-driven camera parallax, per layer.

- **Render live on Metal today**, through wgpu — Vulkan/DX12 come from the
  same wgpu backend without a rewrite, once a consumer needs them.

- **Say plainly what it can't do yet** — see the next section — rather than
  silently drop or guess at anything a real scene needs.

## Technologies

<div style="display: flex; align-items: center; gap: 10px; flex-wrap: wrap;">
  <img src="https://img.shields.io/badge/Rust-black?style=for-the-badge&logo=rust"/>
  <img src="https://img.shields.io/badge/wgpu-black?style=for-the-badge"/>
  <img src="https://img.shields.io/badge/naga-black?style=for-the-badge"/>
  <img src="https://img.shields.io/badge/Metal-black?style=for-the-badge"/>
</div>

## Status: known gaps

Not yet rendered: text, clock and web objects; the `vortex`, `turbulence` and
`controlpointattract` particle operators; sprite trails; textures in container
formats `we_tex` doesn't decode. An audio layer is parsed — asset path, real
per-track volume — but not played: this library draws pixels, and a consumer
plays the audio itself.

Some scenes use Wallpaper Engine's own built-in stock textures, which are not shipped here. Point `WER_ASSETS_DIR` at a folder
holding your own copies (`materials/<name>.tex` or `.png`, Wallpaper Engine's
own layout) and this library will use them.

```
scene/   the scene model, Wallpaper Engine's .pkg and .tex readers, the
         scene.json translator (we_compat), and the shader dialect compiler
engine/  wgpu on Metal: image layers, the effect chain, particles, parallax
video/   a small helper crate the engine depends on
ffi/     the C interface (ffi/include/wer_ffi.h)
```

## Using This From Another App

The public surface is the C ABI in `ffi/include/wer_ffi.h` — link
`libwer_ffi.a` (built by `cargo build --release -p wer-ffi`) and call it from
whatever language can call C. Nothing in here assumes a particular consumer;
Muro is one, linking it as a git submodule and calling it from Swift through a
`.modulemap` (see Muro's own repository for that half, not this one).

## Testing

```bash
cargo test --release
```

## Authors & contributors

The original setup of this repository is by
[Arnaud BEUX](https://github.com/Nytuo)

For a full list of all authors and contributors, see
[the contributors page](https://github.com/Nytuo/wallpaper-engine-rosetta/contributors).

## License

wallpaper-engine-rosetta is licensed under the **GNU General Public License
v3, or later**. It is provided **"as is"** without any **warranty**. Use at
your own risk. See [LICENSE](LICENSE) for more information.
