#![forbid(unsafe_code)]

//! Doom Fire effect.
//!
//! Recreates the classic PSX Doom fire effect using a cellular automaton.
//!
//! Algorithm adapted from Fabien Sanglard's analysis:
//! <https://fabiensanglard.net/doom_fire_psx/>

#[cfg(feature = "canvas")]
use crate::canvas::Painter;
use crate::visual_fx::{BackdropFx, FxContext};
use ftui_render::cell::PackedRgba;
use rand::RngExt;
use rand::rngs::SmallRng;

/// The Doom Fire effect.
pub struct DoomMeltFx {
    /// Heat buffer (width * height).
    heat: Vec<u8>,
    /// Palette cache to avoid recomputing colors.
    palette: [PackedRgba; 37],
    /// Last known width/height to detect resize.
    size: (u16, u16),
    /// Random number generator.
    rng: SmallRng,
}

impl DoomMeltFx {
    pub fn new() -> Self {
        Self {
            heat: Vec::new(),
            palette: Self::build_palette(),
            size: (0, 0),
            rng: rand::make_rng::<SmallRng>(),
        }
    }

    /// Build the classic Doom fire palette (37 colors: black to white).
    fn build_palette() -> [PackedRgba; 37] {
        // Doom Fire Palette (approximate RGB values)
        let colors = [
            (7, 7, 7),
            (31, 7, 7),
            (47, 15, 7),
            (71, 15, 7),
            (87, 23, 7),
            (103, 31, 7),
            (119, 31, 7),
            (143, 39, 7),
            (159, 47, 7),
            (175, 63, 7),
            (191, 71, 7),
            (199, 71, 7),
            (223, 79, 7),
            (223, 87, 7),
            (223, 87, 7),
            (215, 95, 7),
            (215, 95, 7),
            (215, 103, 15),
            (207, 111, 15),
            (207, 119, 15),
            (207, 127, 15),
            (207, 135, 23),
            (199, 135, 23),
            (199, 143, 23),
            (199, 151, 31),
            (191, 159, 31),
            (191, 159, 31),
            (191, 167, 39),
            (191, 167, 39),
            (191, 175, 47),
            (183, 175, 47),
            (183, 183, 47),
            (183, 183, 55),
            (207, 207, 111),
            (223, 223, 159),
            (239, 239, 199),
            (255, 255, 255),
        ];

        let mut palette = [PackedRgba::default(); 37];
        for (i, &(r, g, b)) in colors.iter().enumerate() {
            palette[i] = PackedRgba::rgb(r, g, b);
        }
        palette
    }

    fn resize_buffer(&mut self, width: u16, height: u16) {
        let w = width as usize;
        let h = height as usize;
        let len = w * h;

        if self.heat.len() != len {
            self.heat.resize(len, 0);
            self.size = (width, height);
        }
    }

    fn spread_fire(&mut self, width: usize, height: usize) {
        // Fire logic: iterate from x=0..width, y=0..height
        for x in 0..width {
            for y in 1..height {
                let src_idx = y * width + x;
                let pixel = self.heat[src_idx];

                if pixel == 0 {
                    let dst_idx = (y - 1) * width + x;
                    self.heat[dst_idx] = 0;
                } else {
                    let rand_idx = (self.rng.random::<u8>() & 3) as usize; // 0..3
                    let dst_x =
                        (x as isize - rand_idx as isize + 1).rem_euclid(width as isize) as usize;
                    let dst_idx = (y - 1) * width + dst_x;

                    let new_heat = pixel.saturating_sub(rand_idx as u8 & 1);
                    self.heat[dst_idx] = new_heat;
                }
            }
        }
    }

    /// Render to a sub-pixel painter (high-resolution fire).
    #[cfg(feature = "canvas")]
    pub fn render_painter(&mut self, painter: &mut Painter) {
        let (w, h) = painter.size();
        let width = w as usize;
        let height = h as usize;

        self.resize_buffer(w, h);

        // Fill bottom row
        let last_row_start = (height - 1) * width;
        for i in 0..width {
            self.heat[last_row_start + i] = 36;
        }

        self.spread_fire(width, height);

        for y in 0..height {
            for x in 0..width {
                let idx = y * width + x;
                let heat = self.heat[idx];
                if heat > 0 {
                    let color = self.palette[heat as usize];
                    painter.point_colored(x as i32, y as i32, color);
                }
            }
        }
    }
}

impl BackdropFx for DoomMeltFx {
    fn name(&self) -> &'static str {
        "doom-fire"
    }

    fn resize(&mut self, width: u16, height: u16) {
        self.resize_buffer(width, height);
    }

    fn render(&mut self, ctx: FxContext<'_>, out: &mut [PackedRgba]) {
        if ctx.is_empty() {
            return;
        }

        let width = ctx.width as usize;
        let height = ctx.height as usize;

        // Ensure buffer matches
        self.resize_buffer(ctx.width, ctx.height);

        // Fill bottom row with max heat (source)
        let last_row_start = (height - 1) * width;
        for i in 0..width {
            self.heat[last_row_start + i] = 36;
        }

        // Propagate fire
        self.spread_fire(width, height);

        // Render to output
        for (i, &heat) in self.heat.iter().enumerate() {
            if i < out.len() {
                let color_idx = (heat as usize).min(36);
                out[i] = self.palette[color_idx];
            }
        }
    }
}

impl Default for DoomMeltFx {
    fn default() -> Self {
        Self::new()
    }
}
