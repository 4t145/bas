use super::*;

#[derive(Default)]
pub(in crate::workspace) struct Navigation {
    pub overview: Option<crate::connection::Prepared<bas::service::HistoryOverview>>,
    pub overview_request: Option<u64>,
    pub wanted_revision: u64,
    pub overview_error: Option<Failure>,
    pub location: Option<LocationRequest>,
    pub location_error: Option<Failure>,
    pub after: Option<u64>,
    pub detached: bool,
    pub selected: Option<u64>,
    pub wheel_pixels: f32,
    sequence: u64,
}

#[derive(Clone, Copy)]
pub(in crate::workspace) struct LocationRequest {
    pub request: u64,
    pub cursor: u64,
    pub append: bool,
}

impl Navigation {
    pub fn next_request(&mut self) -> u64 {
        self.sequence += 1;
        self.sequence
    }

    pub fn park(&mut self) {
        self.overview_request = None;
        self.location = None;
        // A cancelled jump cannot remain highlighted over the previous body.
        self.selected = None;
        self.wheel_pixels = 0.;
    }

    pub fn retained_bytes(&self) -> usize {
        self.overview.as_ref().map_or(0, |overview| overview.bytes)
    }
}
