# Third-party notices

## wgpu-hal

`third_party/wgpu-hal` is [wgpu-hal](https://github.com/gfx-rs/wgpu) 30.0.1 by the gfx-rs
developers, with one upstream fix applied
([gfx-rs/wgpu#10124](https://github.com/gfx-rs/wgpu/pull/10124)). It is licensed under MIT or
Apache-2.0; both licenses are in that folder (`LICENSE.MIT`, `LICENSE.APACHE`).

## Pixelify Sans

`crates/horizon-theme/fonts/PixelifySans-*.ttf` is [Pixelify Sans](https://github.com/eifetx/Pixelify-Sans)
by The Pixelify Sans Project Authors, licensed under the SIL Open Font License 1.1
(`crates/horizon-theme/fonts/OFL-PixelifySans.txt`).

## Pixelarticons

`crates/horizon-theme/icons/pixel/*.svg` are icons from
[Pixelarticons](https://github.com/halfmage/pixelarticons) by Gerrit Halfmann, under the MIT
license (`crates/horizon-theme/icons/pixel/LICENSE-pixelarticons.txt`). `star.svg` is Horizon's own, drawn on
the same grid.

## RTX Remix for Minecraft's sky

`crates/horizon-client/src/gfx/sky_atmosphere.wgsl` is a WGSL port of `sky_atmosphere.comp` from
RTX Remix for Minecraft (the renderer's `shaders/sky_atmosphere.comp`), Copyright (c) 2026 NVIDIA
CORPORATION & AFFILIATES, licensed under the Apache License, Version 2.0
(<https://www.apache.org/licenses/LICENSE-2.0>). The file's header says what was ported and what is
Horizon's own (the last pass, which resamples the sky into Horizon's sky-view table).

## The Moon's map

`crates/horizon-client/src/gfx/moon_lroc.png` is NASA's LROC colour map of the Moon from the
[CGI Moon Kit](https://svs.gsfc.nasa.gov/4720/) (NASA's Scientific Visualization Studio, from the
Lunar Reconnaissance Orbiter Camera), scaled to 1024x512. NASA's imagery is in the public domain.
