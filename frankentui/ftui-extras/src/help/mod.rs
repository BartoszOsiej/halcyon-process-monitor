#![forbid(unsafe_code)]

//! Contextual help system with tooltips and guided tours.
//!
//! This module provides tooltip widgets and a guided tour system that
//! integrate with the focus system to display contextual help.
//!
//! # Features
//!
//! - **Tooltips**: Floating help text near focused widgets with configurable
//!   positioning, delay, and auto-dismiss behavior.
//!
//! - **Guided Tours**: Step-by-step onboarding walkthroughs with spotlight
//!   highlighting, progress tracking, and completion persistence.
//!
//! # Example
//!
//! ```ignore
//! use ftui_extras::help::{Tour, TourStep, TourState, Spotlight};
//!
//! // Define a tour
//! let tour = Tour::new("onboarding")
//!     .add_step(TourStep::new("Welcome").content("Let's get started!"))
//!     .add_step(TourStep::new("Search").content("Find items here.").target_widget(1));
//!
//! // Start the tour
//! let mut state = TourState::new();
//! state.start(tour);
//!
//! // Render spotlight for current step
//! if let Some(step) = state.current_step() {
//!     let spotlight = Spotlight::new()
//!         .title(&step.title)
//!         .content(&step.content);
//!     // spotlight.render(...)
//! }
//! ```

mod spotlight;
mod tooltip;
mod tour;

pub use spotlight::{PanelPosition, Spotlight, SpotlightConfig};
pub use tooltip::{Tooltip, TooltipConfig, TooltipPosition, TooltipState};
pub use tour::{
    CompletionStatus, Tour, TourAction, TourCompletion, TourEvent, TourState, TourStep,
};
