//! Where the parks are in the room: spatial anchors (`XR_FB_spatial_entity`),
//! which the headset keeps fixed to the real room.
//!
//! The pieces' poses are in the tracking (STAGE) space, and that space is
//! not the same from one session to the next: its floor is found a few
//! centimetres higher or lower, and it moves altogether when the boundary
//! is redrawn or re-centred. So each park keeps the anchors it was put by
//! ([`super::parks::AnchorRecord`]), and where its frame is from each.
//!
//! Once a session, when the room has been read, the headset is asked for
//! every anchor of every park at once. The ones it finds are in this room:
//! their parks open where they were. Each one found is located every frame,
//! so the open park can move with its anchor ([`super::parks`]). An anchor is
//! made (and stored on the headset) for the open park when it has none here,
//! and anchors of a park deleted, or forgotten here, are erased.
use super::parks::Parks;
use crate::xr::planes::{cvt, enable_locatable};
use bevy::prelude::*;
use bevy_mod_openxr::{
    exts::OxrEnabledExtensions,
    openxr_session_running,
    poll_events::{OxrEventHandlerExt, OxrEventIn},
    resources::{OxrFrameState, OxrInstance},
    session::OxrSession,
};
use bevy_mod_openxr::spaces::OxrSpaceExt;
use bevy_mod_xr::spaces::XrPrimaryReferenceSpace;
use openxr::sys::{self, Handle as _};
use std::collections::HashMap;
use std::ptr;

/// Between tries at making an anchor, s.
const RETRY_SECONDS: f32 = 10.0;

#[derive(Default, Clone, Copy, PartialEq, Debug)]
enum Search {
    /// Not asked yet this session.
    #[default]
    NotYet,
    Running(sys::AsyncRequestIdFB),
    Done,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum MakeStep {
    Making(sys::AsyncRequestIdFB),
    MakingStorable(sys::AsyncRequestIdFB),
    Storing(sys::AsyncRequestIdFB),
}

/// An anchor being made for a park, at a tracking-space point (turned 0).
#[derive(Clone, Debug)]
struct Making {
    park: String,
    at: Vec3,
    step: MakeStep,
    space: Option<sys::Space>,
    uuid: Option<[u8; 16]>,
}

#[derive(Resource, Default)]
pub(crate) struct Anchors {
    /// The headset can keep anchors (none: not known yet).
    supported: Option<bool>,
    search: Search,
    /// The anchors found (or made) this session, by UUID.
    spaces: HashMap<String, sys::Space>,
    /// Where each is in the tracking space, and its turn, when last tracked.
    located: HashMap<String, (Vec3, f32)>,
    /// A park that wants an anchor, and where.
    wanted: Option<(String, Vec3)>,
    making: Option<Making>,
    /// Made and stored: (park, UUID, where, turn), for the parks to take.
    made: Vec<(String, String, Vec3, f32)>,
    /// To be erased from the headset.
    erasing: Vec<String>,
    wait: f32,
    /// Found but not (yet) located, said in the log once.
    unlocated_said: std::collections::HashSet<String>,
}

impl Anchors {
    /// Where anchor `uuid` is in this room, if it is here.
    pub(crate) fn located(&self, uuid: &str) -> Option<(Vec3, f32)> {
        self.located.get(uuid).copied()
    }

    /// The headset may still find more of the parks' anchors.
    pub(crate) fn searching(&self) -> bool {
        self.supported != Some(false) && self.search != Search::Done
    }

    /// A new anchor can be asked for now.
    pub(crate) fn can_make(&self) -> bool {
        self.supported == Some(true) && self.search == Search::Done && self.wanted.is_none() && self.making.is_none() && self.wait <= 0.0
    }

    /// An anchor for `park` at `at` (tracking space), turned as the space is.
    pub(crate) fn make(&mut self, park: String, at: Vec3) {
        self.wanted = Some((park, at));
    }

    /// The anchors made since this was last asked.
    pub(crate) fn take_made(&mut self) -> Vec<(String, String, Vec3, f32)> {
        std::mem::take(&mut self.made)
    }

    /// Anchor `uuid` erased from the headset (if it was found here; else it
    /// is only forgotten).
    pub(crate) fn erase(&mut self, uuid: &str) {
        self.located.remove(uuid);
        if !self.erasing.iter().any(|u| u == uuid) {
            self.erasing.push(uuid.to_string());
        }
    }

    #[cfg(test)]
    pub(crate) fn found_for_test(&mut self, uuids: &[&str]) {
        self.supported = Some(true);
        self.search = Search::Done;
        for u in uuids {
            self.located.insert(u.to_string(), (Vec3::ZERO, 0.0));
        }
    }
}

pub(super) struct AnchorPlugin;

impl Plugin for AnchorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Anchors>()
            .add_oxr_event_handler(on_event)
            .add_systems(PreUpdate, supported)
            .add_systems(PreUpdate, (ask, locate).chain().after(supported).run_if(openxr_session_running).run_if(enabled));
    }
}

fn enabled(exts: Option<Res<OxrEnabledExtensions>>) -> bool {
    exts.is_some_and(|e| e.fb_spatial_entity && e.fb_spatial_entity_query && e.fb_spatial_entity_storage)
}

/// Whether the headset keeps anchors (without them every park is put by hand).
fn supported(exts: Option<Res<OxrEnabledExtensions>>, mut anchors: ResMut<Anchors>) {
    if anchors.supported.is_none() {
        let yes = enabled(exts);
        anchors.supported = Some(yes);
        if !yes {
            info!("PARK ANCHOR the headset keeps no anchors here: every park is put down by hand");
        }
    }
}

fn hex(uuid: &[u8; 16]) -> String {
    uuid.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<[u8; 16]> {
    let mut uuid = [0u8; 16];
    if text.len() != 32 || !text.is_ascii() {
        return None;
    }
    for (i, byte) in uuid.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(uuid)
}

/// Asks the headset for the parks' anchors, makes the one wanted, erases
/// the ones let go.
#[allow(clippy::too_many_arguments)]
fn ask(
    instance: Res<OxrInstance>,
    session: Res<OxrSession>,
    primary: Res<XrPrimaryReferenceSpace>,
    frame: Option<Res<OxrFrameState>>,
    parks: Res<Parks>,
    time: Res<Time<Real>>,
    mut scene: ResMut<crate::xr::planes::SceneQuery>,
    mut anchors: ResMut<Anchors>,
) {
    // One query at a time with the headset: the room's wait while the
    // anchors are looked for, and the anchors' until the room has been read.
    scene.bypass_change_detection().hold(matches!(anchors.search, Search::Running(_)));
    if anchors.wait > 0.0 {
        anchors.wait -= time.delta_secs();
    }
    // Erasing needs nothing else to wait.
    for uuid in std::mem::take(&mut anchors.erasing) {
        let Some(space) = anchors.spaces.remove(&uuid) else { continue };
        let Some(fp) = instance.exts().fb_spatial_entity_storage.as_ref() else { continue };
        let info = sys::SpaceEraseInfoFB { ty: sys::SpaceEraseInfoFB::TYPE, next: ptr::null(), space, location: sys::SpaceStorageLocationFB::LOCAL };
        let mut request = sys::AsyncRequestIdFB::from_raw(0);
        match cvt(unsafe { (fp.erase_space)(session.as_raw(), &info, &mut request) }) {
            Ok(_) => info!("PARK ANCHOR erasing {uuid}"),
            Err(e) => warn!("PARK ANCHOR can't erase {uuid}: {e:?}"),
        }
    }
    if anchors.search == Search::NotYet {
        // (the parks are read at startup; the room first)
        if parks.revision == 0 || !scene.settled() {
            return;
        }
        let mut uuids: Vec<sys::UuidEXT> = parks.list.iter().flat_map(|p| p.file.anchors.iter()).filter_map(|r| unhex(&r.uuid)).map(|data| sys::UuidEXT { data }).collect();
        uuids.sort_by_key(|u| u.data);
        uuids.dedup_by_key(|u| u.data);
        if uuids.is_empty() {
            anchors.search = Search::Done;
            return;
        }
        let Some(fp) = instance.exts().fb_spatial_entity_query.as_ref() else { return };
        // (in the headset's own storage, where they were saved - Meta's sample chains this to the IDs)
        let local = sys::SpaceStorageLocationFilterInfoFB {
            ty: sys::SpaceStorageLocationFilterInfoFB::TYPE,
            next: ptr::null(),
            location: sys::SpaceStorageLocationFB::LOCAL,
        };
        let filter = sys::SpaceUuidFilterInfoFB {
            ty: sys::SpaceUuidFilterInfoFB::TYPE,
            next: (&local as *const sys::SpaceStorageLocationFilterInfoFB).cast(),
            uuid_count: uuids.len() as u32,
            uuids: uuids.as_mut_ptr(),
        };
        let info = sys::SpaceQueryInfoFB {
            ty: sys::SpaceQueryInfoFB::TYPE,
            next: ptr::null(),
            query_action: sys::SpaceQueryActionFB::LOAD,
            max_result_count: uuids.len() as u32,
            timeout: sys::Duration::NONE,
            filter: (&filter as *const sys::SpaceUuidFilterInfoFB).cast(),
            exclude_filter: ptr::null(),
        };
        let mut request = sys::AsyncRequestIdFB::from_raw(0);
        match cvt(unsafe { (fp.query_spaces)(session.as_raw(), (&info as *const sys::SpaceQueryInfoFB).cast(), &mut request) }) {
            Ok(_) => {
                info!("PARK ANCHOR looking for {} anchors of {} parks", uuids.len(), parks.list.len());
                anchors.search = Search::Running(request);
            }
            Err(e) => {
                warn!("PARK ANCHOR the parks' anchors can't be looked for ({e:?}): every park is put down by hand");
                anchors.search = Search::Done;
            }
        }
        return;
    }
    if anchors.making.is_some() || matches!(anchors.search, Search::Running(_)) {
        return;
    }
    let Some((park, at)) = anchors.wanted.take() else { return };
    let (Some(frame), Some(fp)) = (frame, instance.exts().fb_spatial_entity.as_ref()) else {
        anchors.wanted = Some((park, at));
        return;
    };
    let info = sys::SpatialAnchorCreateInfoFB {
        ty: sys::SpatialAnchorCreateInfoFB::TYPE,
        next: ptr::null(),
        space: primary.as_raw_openxr_space(),
        pose_in_space: sys::Posef { orientation: sys::Quaternionf { x: 0.0, y: 0.0, z: 0.0, w: 1.0 }, position: sys::Vector3f { x: at.x, y: at.y, z: at.z } },
        time: frame.predicted_display_time,
    };
    let mut request = sys::AsyncRequestIdFB::from_raw(0);
    match cvt(unsafe { (fp.create_spatial_anchor)(session.as_raw(), &info, &mut request) }) {
        Ok(_) => anchors.making = Some(Making { park, at, step: MakeStep::Making(request), space: None, uuid: None }),
        Err(e) => {
            warn!("PARK ANCHOR can't be made ({e:?}), trying again later");
            anchors.wait = RETRY_SECONDS;
        }
    }
}

fn store(instance: &OxrInstance, session: &OxrSession, anchors: &mut Anchors) {
    let Some(making) = anchors.making.as_mut() else { return };
    let (Some(fp), Some(space)) = (instance.exts().fb_spatial_entity_storage.as_ref(), making.space) else { return };
    let info = sys::SpaceSaveInfoFB {
        ty: sys::SpaceSaveInfoFB::TYPE,
        next: ptr::null(),
        space,
        location: sys::SpaceStorageLocationFB::LOCAL,
        persistence_mode: sys::SpacePersistenceModeFB::INDEFINITE,
    };
    let mut request = sys::AsyncRequestIdFB::from_raw(0);
    match cvt(unsafe { (fp.save_space)(session.as_raw(), &info, &mut request) }) {
        Ok(_) => making.step = MakeStep::Storing(request),
        Err(e) => {
            warn!("PARK ANCHOR can't be stored ({e:?}), trying again later");
            give_up(anchors);
        }
    }
}

/// The anchor being made failed: another is made later.
fn give_up(anchors: &mut Anchors) {
    anchors.making = None;
    anchors.wait = RETRY_SECONDS;
}

fn on_event(event: OxrEventIn, instance: Res<OxrInstance>, session: Option<Res<OxrSession>>, mut anchors: ResMut<Anchors>) {
    let Some(session) = session else { return };
    let making = anchors.making.as_ref().map(|m| m.step);
    match *event {
        openxr::Event::SpaceQueryResultsAvailableFB(e) if anchors.search == Search::Running(e.request_id()) => {
            let Some(fp) = instance.exts().fb_spatial_entity_query.as_ref() else { return };
            // How many, then all of them.
            let mut results = sys::SpaceQueryResultsFB { ty: sys::SpaceQueryResultsFB::TYPE, next: ptr::null_mut(), result_capacity_input: 0, result_count_output: 0, results: ptr::null_mut() };
            if let Err(err) = cvt(unsafe { (fp.retrieve_space_query_results)(session.as_raw(), e.request_id(), &mut results) }) {
                warn!("PARK ANCHOR xrRetrieveSpaceQueryResultsFB: {err:?}");
                return;
            }
            let mut found = vec![sys::SpaceQueryResultFB { space: sys::Space::NULL, uuid: sys::UuidEXT { data: [0; 16] } }; results.result_count_output as usize];
            results.result_capacity_input = found.len() as u32;
            results.results = found.as_mut_ptr();
            match cvt(unsafe { (fp.retrieve_space_query_results)(session.as_raw(), e.request_id(), &mut results) }) {
                Ok(_) => {
                    info!("PARK ANCHOR the search returned {} anchors", results.result_count_output);
                    for result in found.iter().take(results.result_count_output as usize).filter(|r| r.space != sys::Space::NULL) {
                        enable_locatable(&instance, result.space);
                        anchors.spaces.insert(hex(&result.uuid.data), result.space);
                    }
                }
                Err(err) => warn!("PARK ANCHOR xrRetrieveSpaceQueryResultsFB: {err:?}"),
            }
        }
        openxr::Event::SpaceQueryCompleteFB(e) if anchors.search == Search::Running(e.request_id()) => {
            anchors.search = Search::Done;
            info!("PARK ANCHOR search done: {} found in this room ({:?})", anchors.spaces.len(), e.result());
        }
        openxr::Event::SpatialAnchorCreateCompleteFB(e) if making == Some(MakeStep::Making(e.request_id())) => {
            if let Err(err) = cvt(e.result()) {
                warn!("PARK ANCHOR wasn't made ({err:?}), trying again later");
                give_up(&mut anchors);
                return;
            }
            enable_locatable(&instance, e.space());
            if let Some(m) = anchors.making.as_mut() {
                m.space = Some(e.space());
                m.uuid = Some(e.uuid().data);
            }
            info!("PARK ANCHOR made: {}", hex(&e.uuid().data));
            // It has to be storable before it can be stored.
            let Some(fp) = instance.exts().fb_spatial_entity.as_ref() else { return };
            let info = sys::SpaceComponentStatusSetInfoFB {
                ty: sys::SpaceComponentStatusSetInfoFB::TYPE,
                next: ptr::null(),
                component_type: sys::SpaceComponentTypeFB::STORABLE,
                enabled: sys::TRUE,
                timeout: sys::Duration::NONE,
            };
            let mut request = sys::AsyncRequestIdFB::from_raw(0);
            match unsafe { (fp.set_space_component_status)(e.space(), &info, &mut request) } {
                sys::Result::ERROR_SPACE_COMPONENT_STATUS_ALREADY_SET_FB => store(&instance, &session, &mut anchors),
                result => match cvt(result) {
                    Ok(_) => {
                        if let Some(m) = anchors.making.as_mut() {
                            m.step = MakeStep::MakingStorable(request);
                        }
                    }
                    Err(err) => {
                        warn!("PARK ANCHOR can't be made storable ({err:?}), trying again later");
                        give_up(&mut anchors);
                    }
                },
            }
        }
        openxr::Event::SpaceSetStatusCompleteFB(e) if making == Some(MakeStep::MakingStorable(e.request_id())) => {
            if let Err(err) = cvt(e.result()) {
                warn!("PARK ANCHOR wasn't made storable ({err:?}), trying again later");
                give_up(&mut anchors);
                return;
            }
            store(&instance, &session, &mut anchors);
        }
        openxr::Event::SpaceSaveCompleteFB(e) if making == Some(MakeStep::Storing(e.request_id())) => {
            let Some(m) = anchors.making.take() else { return };
            match (cvt(e.result()), m.space, m.uuid) {
                (Ok(_), Some(space), Some(uuid)) => {
                    let name = hex(&uuid);
                    info!("PARK ANCHOR {name} stored on the headset for park {}", m.park);
                    anchors.spaces.insert(name.clone(), space);
                    anchors.located.insert(name.clone(), (m.at, 0.0));
                    anchors.made.push((m.park, name, m.at, 0.0));
                }
                (result, ..) => {
                    warn!("PARK ANCHOR wasn't stored ({result:?}), trying again later");
                    anchors.wait = RETRY_SECONDS;
                }
            }
        }
        _ => {}
    }
}

/// Where every anchor found is in the tracking space now (while it is tracked).
fn locate(instance: Res<OxrInstance>, primary: Res<XrPrimaryReferenceSpace>, frame: Option<Res<OxrFrameState>>, mut anchors: ResMut<Anchors>) {
    let Some(frame) = frame else { return };
    let needed = sys::SpaceLocationFlags::POSITION_VALID
        | sys::SpaceLocationFlags::ORIENTATION_VALID
        | sys::SpaceLocationFlags::POSITION_TRACKED
        | sys::SpaceLocationFlags::ORIENTATION_TRACKED;
    let spaces: Vec<(String, sys::Space)> = anchors.spaces.iter().map(|(u, s)| (u.clone(), *s)).collect();
    for (uuid, space) in spaces {
        let mut location = sys::SpaceLocation { ty: sys::SpaceLocation::TYPE, next: ptr::null_mut(), location_flags: sys::SpaceLocationFlags::EMPTY, pose: sys::Posef::IDENTITY };
        let located = cvt(unsafe { (instance.fp().locate_space)(space, primary.as_raw_openxr_space(), frame.predicted_display_time, &mut location) });
        if located.is_err() || !location.location_flags.contains(needed) {
            // (said once an anchor: found on the headset, but not where it is)
            if !anchors.located.contains_key(&uuid) && anchors.unlocated_said.insert(uuid.clone()) {
                info!("PARK ANCHOR {uuid} found but not located yet: {located:?}, flags {:?}", location.location_flags);
            }
            continue;
        }
        let (p, o) = (location.pose.position, location.pose.orientation);
        let position = Vec3::new(p.x, p.y, p.z);
        let along = Quat::from_xyzw(o.x, o.y, o.z, o.w).normalize() * Vec3::X;
        let yaw = f32::atan2(-along.z, along.x);
        if position.is_finite() && yaw.is_finite() {
            if !anchors.located.contains_key(&uuid) {
                info!("PARK ANCHOR {uuid} is here, at {position:.3?}");
            }
            anchors.located.insert(uuid, (position, yaw));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_uuid_reads_back() {
        let uuid = [0x00, 0x1f, 0xa0, 0xff, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
        assert_eq!(unhex(&hex(&uuid)), Some(uuid));
        assert_eq!(unhex("nope"), None);
    }

    #[test]
    fn the_search_and_making_wait_their_turn() {
        let mut anchors = Anchors::default();
        assert!(anchors.searching() && !anchors.can_make(), "not known yet");
        anchors.supported = Some(false);
        assert!(!anchors.searching() && !anchors.can_make(), "a headset without anchors: nothing to wait for");
        anchors.found_for_test(&["a"]);
        assert!(anchors.can_make() && anchors.located("a").is_some() && anchors.located("b").is_none());
        anchors.make("park".into(), Vec3::ONE);
        assert!(!anchors.can_make(), "one at a time");
        anchors.erase("a");
        assert!(anchors.located("a").is_none());
    }
}
