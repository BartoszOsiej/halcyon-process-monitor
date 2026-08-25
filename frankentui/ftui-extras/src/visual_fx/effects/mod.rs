pub mod doom_fire;
#[cfg(feature = "doom")]
pub mod doom_melt;
pub mod metaballs;
pub mod plasma;
#[cfg(feature = "quake")]
pub mod quake_console;
pub mod sampling;
pub mod screen_melt;
pub mod underwater_warp;

#[cfg(feature = "canvas")]
pub mod canvas_adapters;

pub use doom_fire::DoomFireFx;
#[cfg(feature = "doom")]
pub use doom_melt::DoomMeltFx;
pub use metaballs::{Metaball, MetaballsFx, MetaballsPalette, MetaballsParams};
pub use plasma::{PlasmaFx, PlasmaPalette, plasma_wave, plasma_wave_low};
#[cfg(feature = "quake")]
pub use quake_console::QuakeConsoleFx;
pub use sampling::{
    BallState, CoordCache, FnSampler, MetaballFieldSampler, PlasmaSampler, Sampler,
    cell_to_normalized, fill_normalized_coords,
};
pub use screen_melt::ScreenMeltFx;
pub use underwater_warp::UnderwaterWarpFx;

#[cfg(feature = "canvas")]
pub use canvas_adapters::{MetaballsCanvasAdapter, PlasmaCanvasAdapter};
