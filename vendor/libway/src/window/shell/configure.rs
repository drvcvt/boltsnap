//! Protocol suggestions are selected independently per axis after validation.
use super::types::{Configuration, ConfigurationRevision, Decorations, Tiled, WindowOptions};
use crate::{Error, Result};

pub(super) const MAX_TITLE_BYTES: usize = 1024;
const MAX_APP_ID_BYTES: usize = 256;

/// Fixed-size known toplevel state, not an arbitrary compositor-owned array.
#[derive(Clone, Copy, Default)]
pub(super) struct States {
    suspended: bool,
    maximized: bool,
    fullscreen: bool,
    activated: bool,
    tiled: Tiled,
    /// Written by the decoration object, not by xdg_toplevel configure.
    pub(super) decorations: Decorations,
}

impl States {
    fn parse(states: &[u8]) -> Self {
        let mut parsed = Self::default();
        for value in states
            .as_chunks::<4>()
            .0
            .iter()
            .map(|value| u32::from_ne_bytes(*value))
        {
            match value {
                1 => parsed.maximized = true,
                2 => parsed.fullscreen = true,
                4 => parsed.activated = true,
                5 => parsed.tiled.left = true,
                6 => parsed.tiled.right = true,
                7 => parsed.tiled.top = true,
                8 => parsed.tiled.bottom = true,
                9 => parsed.suspended = true,
                _ => {}
            }
        }
        parsed
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct SuggestedConfiguration {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) states: States,
    /// A popup's placed rectangle for this sequence.
    pub(super) placement: Option<super::popup::PopupPlacement>,
}

impl SuggestedConfiguration {
    pub(super) fn new(width: i32, height: i32, states: &[u8]) -> Result<Self> {
        let width = u32::try_from(width).map_err(|_| Error::InvalidDimensions)?;
        let height = u32::try_from(height).map_err(|_| Error::InvalidDimensions)?;
        if !states.len().is_multiple_of(4) {
            return Err(Error::InvalidInput("malformed xdg_toplevel state array"));
        }
        Ok(Self {
            width,
            height,
            states: States::parse(states),
            placement: None,
        })
    }

    /// A layer suggestion: a size and no toplevel states.
    pub(super) fn sized(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            states: States::default(),
            placement: None,
        }
    }

    pub(super) fn select(
        self,
        preferred: (u32, u32),
        accepted: Option<Configuration>,
        revision: ConfigurationRevision,
        scale_120: u32,
    ) -> Result<Configuration> {
        let fallback = accepted.map_or(preferred, |value| (value.width, value.height));
        let width = if self.width == 0 {
            fallback.0
        } else {
            self.width
        };
        let height = if self.height == 0 {
            fallback.1
        } else {
            self.height
        };
        let states = self.states;
        Ok(Configuration {
            width,
            height,
            pixel_width: super::scale::pixel_extent(width, scale_120)?,
            pixel_height: super::scale::pixel_extent(height, scale_120)?,
            scale_120,
            suspended: states.suspended,
            decorations: states.decorations,
            maximized: states.maximized,
            fullscreen: states.fullscreen,
            activated: states.activated,
            tiled: states.tiled,
            revision,
            placement: self.placement,
        })
    }
}

/// Only the scale and revision change; every state flag carries over unchanged.
pub(super) fn rescale(
    accepted: Configuration,
    scale_120: u32,
    revision: ConfigurationRevision,
) -> Result<Configuration> {
    Ok(Configuration {
        pixel_width: super::scale::pixel_extent(accepted.width, scale_120)?,
        pixel_height: super::scale::pixel_extent(accepted.height, scale_120)?,
        scale_120,
        revision,
        // A scale-only change keeps the placement but completes no reposition.
        placement: accepted
            .placement
            .map(|placement| super::popup::PopupPlacement {
                repositioned: None,
                ..placement
            }),
        ..accepted
    })
}

pub(super) fn validate_options(options: &WindowOptions) -> Result<()> {
    if options.preferred_width == 0
        || options.preferred_height == 0
        || options.preferred_width > i32::MAX as u32
        || options.preferred_height > i32::MAX as u32
    {
        return Err(Error::InvalidDimensions);
    }
    validate_title(&options.title)?;
    validate_text(
        &options.app_id,
        MAX_APP_ID_BYTES,
        "invalid application identity",
    )
}

pub(super) fn validate_title(title: &str) -> Result<()> {
    validate_text(title, MAX_TITLE_BYTES, "invalid window title")
}

fn validate_text(value: &str, maximum: usize, reason: &'static str) -> Result<()> {
    if value.as_bytes().contains(&0) {
        return Err(Error::InvalidInput(reason));
    }
    if value.len() > maximum {
        return Err(Error::LimitExceeded);
    }
    Ok(())
}
