//! XKB extension wrappers (group get/set, state notifications).

use x11rb::{
    connection::Connection,
    protocol::{
        xkb::{
            self, ConnectionExt as _, EventType, Group as XkbGroup, MapPart, SelectEventsAux, ID,
        },
        xproto::ModMask,
    },
};
use xxkb_core::layout::Group;

use crate::errors::X11Error;

/// XKB-related state owned by the worker thread.
pub struct XkbConn {
    /// Maximum number of groups the server reports.
    pub max_groups: u8,
}

impl XkbConn {
    /// Initialize XKB on the connection: query version, subscribe to state
    /// change events.
    pub fn init<C: Connection>(conn: &C) -> Result<Self, X11Error> {
        let reply = conn
            .xkb_use_extension(1, 0)
            .map_err(|e| X11Error::Other(format!("xkb use_extension: {e}")))?
            .reply()
            .map_err(|e| X11Error::Other(format!("xkb use_extension reply: {e}")))?;
        if !reply.supported {
            return Err(X11Error::MissingExtension("XKB"));
        }

        // Subscribe to state notify, new keyboard notify and map notify.
        let select_events_mask: EventType =
            EventType::STATE_NOTIFY | EventType::NEW_KEYBOARD_NOTIFY | EventType::MAP_NOTIFY;
        // Mask of events to *clear* before our select - none.
        let clear: EventType = EventType::from(0u16);
        // Use the default empty aux (no per-event detail filtering).
        let aux = SelectEventsAux::new();

        let map_part = MapPart::KEY_TYPES | MapPart::KEY_SYMS | MapPart::MODIFIER_MAP;

        conn.xkb_select_events(
            ID::USE_CORE_KBD.into(),
            clear,
            select_events_mask,
            map_part,
            map_part,
            &aux,
        )
        .map_err(|e| X11Error::Other(format!("xkb select_events: {e}")))?;
        conn.flush()
            .map_err(|e| X11Error::Other(format!("xkb flush: {e}")))?;

        let max_groups = max_groups(conn)?;
        Ok(Self { max_groups })
    }
}

/// Read the current group (0-based) from the server.
pub fn current_group<C: Connection>(conn: &C) -> Result<Group, X11Error> {
    let reply = conn
        .xkb_get_state(ID::USE_CORE_KBD.into())
        .map_err(|e| X11Error::Other(format!("xkb get_state: {e}")))?
        .reply()
        .map_err(|e| X11Error::Other(format!("xkb get_state reply: {e}")))?;
    let max = max_groups(conn).unwrap_or(4);
    let g_idx: u8 = reply.group.into();
    Group::new(g_idx, max).map_err(|e| X11Error::Other(format!("group: {e}")))
}

/// Tell the server to lock the group to `g`.
pub fn set_group<C: Connection>(conn: &C, g: Group) -> Result<(), X11Error> {
    let group = match g.as_index() {
        0 => XkbGroup::M1,
        1 => XkbGroup::M2,
        2 => XkbGroup::M3,
        _ => XkbGroup::M4,
    };
    conn.xkb_latch_lock_state(
        ID::USE_CORE_KBD.into(),
        ModMask::from(0u16), // affect_mod_locks
        ModMask::from(0u16), // mod_locks
        true,                // lock_group
        group,               // group_lock
        ModMask::from(0u16), // affect_mod_latches
        false,               // latch_group
        0u16,                // group_latch
    )
    .map_err(|e| X11Error::Other(format!("xkb latch_lock: {e}")))?;
    conn.flush()
        .map_err(|e| X11Error::Other(format!("xkb flush: {e}")))?;
    Ok(())
}

/// Query how many groups the current XKB map has.
///
/// XKB hard-caps the group count at 4. We currently always return 4
/// since there is no single "n_groups" reply value: deriving it from
/// `xkb_get_map` would require reading all the key types. Returning the
/// max is safe — `LayoutState` clamps to whatever is actually in use.
pub fn max_groups<C: Connection>(_conn: &C) -> Result<u8, X11Error> {
    let _ = xkb::NameDetail::GROUP_NAMES; // keep the symbol used
    Ok(4)
}
