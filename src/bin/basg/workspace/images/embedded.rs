//! Ownership of embedded assets rendered directly by TextView rather than ImageCache.

use super::*;
use gpui_kit::{Global, ImageSource};

#[derive(Default)]
struct SharedOwners(HashMap<Arc<Image>, HashSet<EntityId>>);
impl Global for SharedOwners {}

#[derive(Default)]
pub(super) struct Owners {
    owner: Option<EntityId>,
    images: HashSet<Arc<Image>>,
    uris: HashSet<SharedUri>,
}

impl Owners {
    pub(super) fn uris(&self) -> Vec<SharedUri> {
        self.uris.iter().cloned().collect()
    }

    pub(super) fn update(&mut self, references: &HashSet<Resource>, owner: EntityId, cx: &mut App) {
        let uris: HashSet<_> = references
            .iter()
            .filter_map(|resource| match resource {
                Resource::Uri(uri) if uri.starts_with("data:") => Some(uri.clone()),
                _ => None,
            })
            .collect();
        if self.owner == Some(owner) && self.uris == uris {
            return;
        }
        let images: HashSet<_> = uris
            .iter()
            .filter_map(|uri| decode_data_uri(uri).ok().map(Arc::new))
            .collect();
        if !cx.has_global::<SharedOwners>() {
            cx.set_global(SharedOwners::default());
        }
        for image in images.difference(&self.images) {
            cx.global_mut::<SharedOwners>()
                .0
                .entry(image.clone())
                .or_default()
                .insert(owner);
        }
        for image in self.images.difference(&images) {
            release(image, owner, cx);
        }
        self.images = images;
        self.uris = uris;
        self.owner = Some(owner);
    }

    pub(super) fn release(&mut self, cx: &mut App) {
        if let Some(owner) = self.owner.take() {
            for image in self.images.drain() {
                release(&image, owner, cx);
            }
        }
        self.uris.clear();
    }
}

fn release(image: &Arc<Image>, owner: EntityId, cx: &mut App) {
    let owners = &mut cx.global_mut::<SharedOwners>().0;
    let last = owners.get_mut(image).is_some_and(|users| {
        users.remove(&owner);
        users.is_empty()
    });
    if last {
        owners.remove(image);
        ImageSource::Image(image.clone()).remove_asset(cx);
    }
}
