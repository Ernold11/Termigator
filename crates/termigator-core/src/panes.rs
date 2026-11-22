//! Split-pane tree, tabs and the `last_session` snapshot (split, remove,
//! detach, snapshot and restore).
//!
//! A pure structure, no Qt and no I/O (ADR-10): QML gets it as JSON ([`Panes::to_json`]) and builds
//! its `SplitView` recursively. Tabs are opaque [`TabId`]s; titles, connections and sessions live
//! with the caller, which hands them in through a closure when a snapshot is taken.
//!
//! Model:
//! - the root is always a splitter (initially horizontal) holding the permanent *main* pane;
//! - splitting a pane inside a splitter of the same orientation inserts a sibling (its size share is
//!   halved), the other orientation wraps the pane in a new two-child splitter;
//! - a pane that runs out of tabs is removed (the main flag moves to another pane) and a splitter
//!   left with one child (other than the root) is replaced by that child; only the last pane stays;
//! - panes are listed depth-first; exactly one is *active* (the one new tabs and splits go to);
//! - a detached window holds tabs of its own and vanishes with its last tab.
//!
//! Sizes are the splitter sizes the view reported through [`Panes::set_sizes`]; Qt computes them from
//! geometry, so only the rules below are reproduced (halving, `max(200, extent)`
//! for a new nested splitter, the removed pane's share going to its neighbour).
//!
//! [`Panes::snapshot`] writes the exact `last_session` shape (`{split, sizes, children}` /
//! `{tabs, current, main}` + `detached`), never a password; [`restore`] reads it back and says which
//! tabs to open, matching saved connections by `(host, user, port, label)`. RDP, VNC and Files tabs
//! are not restored into tabs.

use serde_json::{Value as J, json};

use crate::config::doc::Value;
use crate::config::model::{
    PROTOCOL_SERIAL, PROTOCOL_SSH, PROTOCOL_TELNET, connection_protocol, strip_space, truthy,
    value_str,
};

/// Opaque tab identifier, allocated by [`Panes::new_tab_id`].
pub type TabId = u64;
/// Pane identifier; the main pane of a new tree is `0`.
pub type PaneId = u64;
/// Detached-window identifier.
pub type WindowId = u64;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PaneError {
    #[error("no such pane: {0}")]
    NoSuchPane(PaneId),
    #[error("no such tab: {0}")]
    NoSuchTab(TabId),
    #[error("tab {0} is not in a detached window")]
    NotDetached(TabId),
    #[error("no such window: {0}")]
    NoSuchWindow(WindowId),
    #[error("tab index out of range")]
    BadIndex,
    #[error("no such splitter")]
    BadPath,
    #[error("a detached window takes tabs only in its middle (no splits)")]
    BadDock,
}

/// Where a dragged tab lands relative to a pane: the middle puts it into the pane's tab
/// list, a side splits the pane and gives the new half to the tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dock {
    Center,
    Left,
    Right,
    Top,
    Bottom,
}

impl Dock {
    /// Parses the QML name (`"center"`, `"left"`, `"right"`, `"top"`, `"bottom"`).
    pub fn parse(name: &str) -> Option<Dock> {
        Some(match name {
            "center" => Dock::Center,
            "left" => Dock::Left,
            "right" => Dock::Right,
            "top" => Dock::Top,
            "bottom" => Dock::Bottom,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Dock::Center => "center",
            Dock::Left => "left",
            Dock::Right => "right",
            Dock::Top => "top",
            Dock::Bottom => "bottom",
        }
    }

    /// Left and right put panes side by side, top and bottom stack them; the middle has none.
    fn orientation(self) -> Option<Orientation> {
        match self {
            Dock::Left | Dock::Right => Some(Orientation::Horizontal),
            Dock::Top | Dock::Bottom => Some(Orientation::Vertical),
            Dock::Center => None,
        }
    }

    /// The new pane goes before (left of / above) the target.
    fn before(self) -> bool {
        matches!(self, Dock::Left | Dock::Top)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// Panes side by side (`"h"`).
    Horizontal,
    /// Panes stacked (`"v"`).
    Vertical,
}

impl Orientation {
    fn code(self) -> &'static str {
        match self {
            Self::Horizontal => "h",
            Self::Vertical => "v",
        }
    }
}

/// Where a tab lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    Pane(PaneId),
    Window(WindowId),
}

/// Tabs with a current index, behaving like `QTabWidget`.
#[derive(Debug, Clone, Default)]
struct TabList {
    tabs: Vec<TabId>,
    current: Option<usize>,
}

impl TabList {
    fn insert(&mut self, at: Option<usize>, tab: TabId) -> usize {
        let i = at.map_or(self.tabs.len(), |a| a.min(self.tabs.len()));
        self.tabs.insert(i, tab);
        self.current = match self.current {
            None => Some(0),
            Some(c) if i <= c => Some(c + 1),
            keep => keep,
        };
        i
    }

    fn remove(&mut self, i: usize) {
        self.tabs.remove(i);
        self.current = match self.current {
            _ if self.tabs.is_empty() => None,
            Some(c) if i < c => Some(c - 1),
            Some(c) => Some(c.min(self.tabs.len() - 1)),
            None => None,
        };
    }

    fn position(&self, tab: TabId) -> Option<usize> {
        self.tabs.iter().position(|&t| t == tab)
    }
}

#[derive(Debug, Clone)]
struct Pane {
    id: PaneId,
    main: bool,
    list: TabList,
}

#[derive(Debug, Clone)]
struct Window {
    id: WindowId,
    list: TabList,
}

#[derive(Debug, Clone)]
enum Node {
    Pane(PaneId),
    Split(Split),
}

#[derive(Debug, Clone)]
struct Split {
    orientation: Orientation,
    sizes: Vec<i64>,
    children: Vec<Node>,
}

fn split_mut<'a>(root: &'a mut Split, path: &[usize]) -> Option<&'a mut Split> {
    let mut cur = root;
    for &i in path {
        match cur.children.get_mut(i)? {
            Node::Split(s) => cur = s,
            Node::Pane(_) => return None,
        }
    }
    Some(cur)
}

fn path_to(split: &Split, pane: PaneId, prefix: &mut Vec<usize>) -> bool {
    for (i, child) in split.children.iter().enumerate() {
        prefix.push(i);
        let found = match child {
            Node::Pane(p) => *p == pane,
            Node::Split(s) => path_to(s, pane, prefix),
        };
        if found {
            return true;
        }
        prefix.pop();
    }
    false
}

fn collect(split: &Split, out: &mut Vec<PaneId>) {
    for child in &split.children {
        match child {
            Node::Pane(p) => out.push(*p),
            Node::Split(s) => collect(s, out),
        }
    }
}

/// The pane tree, its tabs, the active pane and the detached windows.
#[derive(Debug, Clone)]
pub struct Panes {
    root: Split,
    panes: Vec<Pane>,
    windows: Vec<Window>,
    active: PaneId,
    next_pane: u64,
    next_tab: u64,
    next_window: u64,
}

impl Default for Panes {
    fn default() -> Self {
        Self::new()
    }
}

impl Panes {
    /// A window with the single main pane (id `0`), active and empty.
    pub fn new() -> Self {
        Self {
            root: Split {
                orientation: Orientation::Horizontal,
                sizes: vec![0],
                children: vec![Node::Pane(0)],
            },
            panes: vec![Pane {
                id: 0,
                main: true,
                list: TabList::default(),
            }],
            windows: Vec::new(),
            active: 0,
            next_pane: 1,
            next_tab: 1,
            next_window: 1,
        }
    }

    /// A fresh tab id for [`Panes::add_tab`].
    pub fn new_tab_id(&mut self) -> TabId {
        self.next_tab += 1;
        self.next_tab - 1
    }

    pub fn main_pane(&self) -> PaneId {
        self.panes.iter().find(|p| p.main).map_or(0, |p| p.id)
    }

    /// The active pane (`_focused_pane`); always one that exists.
    pub fn active(&self) -> PaneId {
        self.active
    }

    /// Make `pane` active (`_set_terminal_tabs_focus`).
    pub fn set_active(&mut self, pane: PaneId) -> Result<(), PaneError> {
        self.pane(pane)?;
        self.active = pane;
        Ok(())
    }

    /// All panes depth-first (left to right, top to bottom).
    pub fn list_panes(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        collect(&self.root, &mut out);
        out
    }

    pub fn tabs(&self, pane: PaneId) -> Result<&[TabId], PaneError> {
        Ok(&self.pane(pane)?.list.tabs)
    }

    /// Index of the pane's current tab (`None` when empty).
    pub fn current(&self, pane: PaneId) -> Result<Option<usize>, PaneError> {
        Ok(self.pane(pane)?.list.current)
    }

    pub fn windows(&self) -> Vec<WindowId> {
        self.windows.iter().map(|w| w.id).collect()
    }

    pub fn window_tabs(&self, window: WindowId) -> Option<&[TabId]> {
        self.windows
            .iter()
            .find(|w| w.id == window)
            .map(|w| w.list.tabs.as_slice())
    }

    pub fn locate(&self, tab: TabId) -> Option<Location> {
        self.panes
            .iter()
            .find(|p| p.list.position(tab).is_some())
            .map(|p| Location::Pane(p.id))
            .or_else(|| {
                self.windows
                    .iter()
                    .find(|w| w.list.position(tab).is_some())
                    .map(|w| Location::Window(w.id))
            })
    }

    fn pane(&self, id: PaneId) -> Result<&Pane, PaneError> {
        self.panes
            .iter()
            .find(|p| p.id == id)
            .ok_or(PaneError::NoSuchPane(id))
    }

    fn pane_mut(&mut self, id: PaneId) -> Result<&mut Pane, PaneError> {
        self.panes
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or(PaneError::NoSuchPane(id))
    }

    fn list_mut(&mut self, loc: Location) -> Option<&mut TabList> {
        match loc {
            Location::Pane(id) => self
                .panes
                .iter_mut()
                .find(|p| p.id == id)
                .map(|p| &mut p.list),
            Location::Window(id) => self
                .windows
                .iter_mut()
                .find(|w| w.id == id)
                .map(|w| &mut w.list),
        }
    }

    /// Add `tab` to `pane` (at `at`, default the end), make it current and the pane active.
    pub fn add_tab(
        &mut self,
        pane: PaneId,
        tab: TabId,
        at: Option<usize>,
    ) -> Result<(), PaneError> {
        let list = &mut self.pane_mut(pane)?.list;
        let i = list.insert(at, tab);
        list.current = Some(i);
        self.active = pane;
        Ok(())
    }

    /// [`Panes::add_tab`] for a detached window (a new or duplicated tab opens there; a
    /// restart keeps its slot). The active pane does not change.
    pub fn add_window_tab(
        &mut self,
        window: WindowId,
        tab: TabId,
        at: Option<usize>,
    ) -> Result<(), PaneError> {
        let list = self
            .list_mut(Location::Window(window))
            .ok_or(PaneError::NoSuchWindow(window))?;
        let i = list.insert(at, tab);
        list.current = Some(i);
        Ok(())
    }

    /// Make tab `index` of a detached window current.
    pub fn set_window_current(&mut self, window: WindowId, index: usize) -> Result<(), PaneError> {
        let list = self
            .list_mut(Location::Window(window))
            .ok_or(PaneError::NoSuchWindow(window))?;
        if index >= list.tabs.len() {
            return Err(PaneError::BadIndex);
        }
        list.current = Some(index);
        Ok(())
    }

    /// Make tab `index` of `pane` current; the pane becomes active.
    pub fn set_current(&mut self, pane: PaneId, index: usize) -> Result<(), PaneError> {
        let p = self.pane_mut(pane)?;
        if index >= p.list.tabs.len() {
            return Err(PaneError::BadIndex);
        }
        p.list.current = Some(index);
        self.active = pane;
        Ok(())
    }

    /// Puts a new, empty pane next to `pane` (after it, or before it with `before`) in a splitter
    /// of `orientation`: a sibling when the parent already has that orientation (the target's size
    /// is halved), else the target is wrapped in a new two-child splitter of `max(200, extent)`.
    fn insert_pane(
        &mut self,
        pane: PaneId,
        orientation: Orientation,
        before: bool,
        extent: i64,
    ) -> Result<PaneId, PaneError> {
        self.pane(pane)?;
        let mut path = Vec::new();
        if !path_to(&self.root, pane, &mut path) {
            return Err(PaneError::NoSuchPane(pane));
        }
        let (&i, parent_path) = path.split_last().ok_or(PaneError::BadPath)?;
        let new = self.next_pane;
        self.next_pane += 1;
        let parent = split_mut(&mut self.root, parent_path).ok_or(PaneError::BadPath)?;
        if parent.orientation == orientation {
            let at = if before { i } else { i + 1 };
            parent.children.insert(at, Node::Pane(new));
            if let Some(&s) = parent.sizes.get(i) {
                parent.sizes[i] = (s + 1) / 2;
                parent.sizes.insert(at, s / 2);
            } else {
                parent.sizes.push(0);
            }
        } else {
            let total = extent.max(200);
            let children = if before {
                vec![Node::Pane(new), Node::Pane(pane)]
            } else {
                vec![Node::Pane(pane), Node::Pane(new)]
            };
            parent.children[i] = Node::Split(Split {
                orientation,
                sizes: vec![(total + 1) / 2, total / 2],
                children,
            });
        }
        self.panes.push(Pane {
            id: new,
            main: false,
            list: TabList::default(),
        });
        Ok(new)
    }

    /// Split `pane` (the caller passes the focused one). Returns the new pane, which
    /// becomes active. With `move_current_tab` and at least two tabs the pane's current tab moves
    /// into the new pane. `extent` is the size along the new splitter that a nested split divides
    /// (used as `max(200, extent)`); it is ignored when the split extends an existing splitter.
    pub fn split(
        &mut self,
        pane: PaneId,
        orientation: Orientation,
        move_current_tab: bool,
        extent: i64,
    ) -> Result<PaneId, PaneError> {
        let new = self.insert_pane(pane, orientation, false, extent)?;
        let src = &mut self.pane_mut(pane)?.list;
        let moving = if move_current_tab && src.tabs.len() >= 2 {
            src.current.map(|c| {
                let t = src.tabs[c];
                src.remove(c);
                t
            })
        } else {
            None
        };
        if let Some(t) = moving {
            self.add_tab(new, t, None)?;
        }
        self.active = new;
        Ok(new)
    }

    /// Set the sizes of the splitter at `path` (child indices from the root; `[]` is the root);
    /// ignored unless there is one size per child (as `_restore_session` does).
    pub fn set_sizes(&mut self, path: &[usize], sizes: Vec<i64>) -> Result<(), PaneError> {
        let s = split_mut(&mut self.root, path).ok_or(PaneError::BadPath)?;
        if sizes.len() == s.children.len() {
            s.sizes = sizes;
        }
        Ok(())
    }

    /// Close a tab (`_close_terminal_tab_at`). An emptied non-main pane is removed (and splitters
    /// unwrapped) and an emptied detached window forgotten, unless `keep_pane`.
    pub fn close_tab(&mut self, tab: TabId, keep_pane: bool) -> Result<(), PaneError> {
        let loc = self.locate(tab).ok_or(PaneError::NoSuchTab(tab))?;
        self.take_tab(loc, tab);
        if !keep_pane {
            self.remove_if_empty(loc);
        }
        Ok(())
    }

    /// Close every tab of `pane` and remove it (`_close_current_pane`); the main pane is only
    /// emptied. Returns the closed tabs so the caller can end their sessions.
    pub fn close_pane(&mut self, pane: PaneId) -> Result<Vec<TabId>, PaneError> {
        let p = self.pane_mut(pane)?;
        let closed = std::mem::take(&mut p.list.tabs);
        p.list.current = None;
        self.remove_pane(pane);
        Ok(closed)
    }

    /// Move a tab into a window of its own (`_detach_tab`); an emptied source is removed.
    pub fn detach_tab(&mut self, tab: TabId) -> Result<WindowId, PaneError> {
        let loc = self.locate(tab).ok_or(PaneError::NoSuchTab(tab))?;
        self.take_tab(loc, tab);
        let id = self.next_window;
        self.next_window += 1;
        self.windows.push(Window {
            id,
            list: TabList {
                tabs: vec![tab],
                current: Some(0),
            },
        });
        self.remove_if_empty(loc);
        Ok(id)
    }

    /// Move a tab of a detached window back into the active pane as its current tab
    /// (`_attach_tab`); an emptied window is forgotten.
    pub fn attach_tab(&mut self, tab: TabId) -> Result<(), PaneError> {
        let loc = match self.locate(tab) {
            Some(l @ Location::Window(_)) => l,
            Some(Location::Pane(_)) => return Err(PaneError::NotDetached(tab)),
            None => return Err(PaneError::NoSuchTab(tab)),
        };
        self.take_tab(loc, tab);
        self.remove_if_empty(loc);
        self.add_tab(self.active, tab, None)
    }

    /// Drops `tab` onto `to`. `Dock::Center` puts it into the pane's (or window's) tab list at
    /// `at` (final index, default the end; inside its own list this reorders) and makes it
    /// current; a side splits the target pane and the tab takes the new half (`extent` as in
    /// [`Panes::split`]). A detached window accepts only `Center`. A pane's only tab dropped on a
    /// side of its own pane changes nothing. An emptied source is removed. Returns where the tab
    /// is now; a split target becomes the active pane.
    pub fn move_tab(
        &mut self,
        tab: TabId,
        to: Location,
        dock: Dock,
        at: Option<usize>,
        extent: i64,
    ) -> Result<Location, PaneError> {
        let src = self.locate(tab).ok_or(PaneError::NoSuchTab(tab))?;
        match (to, dock.orientation()) {
            (Location::Window(w), None) => {
                self.list_mut(to).ok_or(PaneError::NoSuchWindow(w))?;
            }
            (Location::Window(_), Some(_)) => return Err(PaneError::BadDock),
            (Location::Pane(p), _) => {
                self.pane(p)?;
            }
        }
        let Some(orientation) = dock.orientation() else {
            if src == to {
                let list = self.list_mut(to).ok_or(PaneError::BadPath)?;
                let from = list.position(tab).ok_or(PaneError::NoSuchTab(tab))?;
                list.remove(from);
                let i = list.insert(at.map(|a| a.min(list.tabs.len())), tab);
                list.current = Some(i);
            } else {
                self.take_tab(src, tab);
                match to {
                    Location::Pane(p) => self.add_tab(p, tab, at)?,
                    Location::Window(w) => self.add_window_tab(w, tab, at)?,
                }
                self.remove_if_empty(src);
            }
            return Ok(to);
        };
        let Location::Pane(target) = to else {
            return Err(PaneError::BadDock);
        };
        if src == to && self.pane(target)?.list.tabs.len() == 1 {
            return Ok(to);
        }
        let new = self.insert_pane(target, orientation, dock.before(), extent)?;
        self.take_tab(src, tab);
        self.add_tab(new, tab, None)?;
        self.remove_if_empty(src);
        Ok(Location::Pane(new))
    }

    /// Docks `tab` at an outer edge of the whole pane area (`dock` is a side, never
    /// `Center`): the new pane takes half of `extent` along the root splitter, which is extended
    /// (same orientation, or a root with one child) or wrapped in a new root. Returns the new pane.
    pub fn dock_at_edge(
        &mut self,
        tab: TabId,
        dock: Dock,
        extent: i64,
    ) -> Result<PaneId, PaneError> {
        let orientation = dock.orientation().ok_or(PaneError::BadDock)?;
        let src = self.locate(tab).ok_or(PaneError::NoSuchTab(tab))?;
        if let Location::Pane(p) = src
            && self.panes.len() == 1
            && self.pane(p)?.list.tabs.len() == 1
        {
            return Ok(p);
        }
        let new = self.next_pane;
        self.next_pane += 1;
        let half = extent.max(200) / 2;
        let before = dock.before();
        if self.root.children.len() == 1 || self.root.orientation == orientation {
            self.root.orientation = orientation;
            let at = if before { 0 } else { self.root.children.len() };
            self.root.children.insert(at, Node::Pane(new));
            self.root.sizes.resize(self.root.children.len() - 1, half);
            self.root.sizes.insert(at, half);
        } else {
            let old = std::mem::replace(
                &mut self.root,
                Split {
                    orientation,
                    sizes: Vec::new(),
                    children: Vec::new(),
                },
            );
            let (children, sizes) = if before {
                (vec![Node::Pane(new), Node::Split(old)], vec![half, half])
            } else {
                (vec![Node::Split(old), Node::Pane(new)], vec![half, half])
            };
            self.root.children = children;
            self.root.sizes = sizes;
        }
        self.panes.push(Pane {
            id: new,
            main: false,
            list: TabList::default(),
        });
        self.take_tab(src, tab);
        self.add_tab(new, tab, None)?;
        self.remove_if_empty(src);
        Ok(new)
    }

    fn take_tab(&mut self, loc: Location, tab: TabId) {
        if let Some(list) = self.list_mut(loc)
            && let Some(i) = list.position(tab)
        {
            list.remove(i);
        }
    }

    fn remove_if_empty(&mut self, loc: Location) {
        match loc {
            Location::Pane(id) => {
                let empty = self.pane(id).is_ok_and(|p| p.list.tabs.is_empty());
                if empty {
                    self.remove_pane(id);
                }
            }
            Location::Window(id) => self
                .windows
                .retain(|w| w.id != id || !w.list.tabs.is_empty()),
        }
    }

    /// `_remove_pane`: drop an (empty) pane and unwrap splitters left with one child.
    fn remove_pane(&mut self, pane: PaneId) {
        // The last pane stays, any other (main included) can go.
        if self.panes.len() <= 1 {
            return;
        }
        let mut path = Vec::new();
        if !path_to(&self.root, pane, &mut path) {
            return;
        }
        let Some((&i, parent_path)) = path.split_last() else {
            return;
        };
        if let Some(parent) = split_mut(&mut self.root, parent_path) {
            parent.children.remove(i);
            let share = parent
                .sizes
                .remove(i.min(parent.sizes.len().saturating_sub(1)));
            if let Some(neighbour) = parent.sizes.get_mut(i.saturating_sub(1)) {
                *neighbour += share;
            }
        }
        self.panes.retain(|p| p.id != pane);
        if !self.panes.iter().any(|p| p.main)
            && let Some(p) = self.panes.first_mut()
        {
            p.main = true;
        }
        let mut cur: &[usize] = parent_path;
        while let Some((&j, gp_path)) = cur.split_last() {
            let only_child = split_mut(&mut self.root, cur)
                .filter(|s| s.children.len() == 1)
                .and_then(|s| s.children.pop());
            let Some(child) = only_child else { break };
            if let Some(slot) =
                split_mut(&mut self.root, gp_path).and_then(|gp| gp.children.get_mut(j))
            {
                *slot = child;
            }
            cur = gp_path;
        }
        if !self.list_panes().contains(&self.active) {
            self.active = self.list_panes().first().copied().unwrap_or(0);
        }
    }

    /// The tree as JSON for QML:
    /// `{"root": node, "active": id, "detached": [{"id", "tabs", "current"}]}` with nodes
    /// `{"type": "split", "orientation": "h"|"v", "sizes": [...], "children": [...]}` and
    /// `{"type": "pane", "id", "main", "tabs": [ids], "current": index|null}`.
    pub fn to_json(&self) -> J {
        json!({
            "root": self.split_json(&self.root),
            "active": self.active,
            "detached": self.windows.iter().map(|w| json!({
                "id": w.id, "tabs": w.list.tabs, "current": w.list.current,
            })).collect::<Vec<_>>(),
        })
    }

    fn split_json(&self, s: &Split) -> J {
        let kids: Vec<J> = s
            .children
            .iter()
            .map(|c| match c {
                Node::Split(s) => self.split_json(s),
                Node::Pane(id) => self
                    .panes
                    .iter()
                    .find(|p| p.id == *id)
                    .map_or(J::Null, |p| {
                        json!({"type": "pane", "id": p.id, "main": p.main,
                           "tabs": p.list.tabs, "current": p.list.current})
                    }),
            })
            .collect();
        json!({"type": "split", "orientation": s.orientation.code(), "sizes": s.sizes,
               "children": kids})
    }

    /// `_session_snapshot`. `record` describes an embedded terminal tab, `None` for any other
    /// (Files, …) which is left out.
    pub fn snapshot(&self, record: &dyn Fn(TabId) -> Option<TabRecord>) -> Value {
        let rec = |t: &TabId| record(*t).map(|r| r.to_value());
        let mut snap = self
            .snap_split(&self.root, &rec)
            .unwrap_or_else(|| leaf_value(Vec::new(), 0, true));
        let detached: Vec<Value> = self
            .windows
            .iter()
            .flat_map(|w| w.list.tabs.iter().filter_map(&rec))
            .collect();
        if !detached.is_empty()
            && let Value::Map(m) = &mut snap
        {
            m.push((Value::Str("detached".into()), Value::Seq(detached)));
        }
        snap
    }

    fn snap_split(&self, s: &Split, rec: &dyn Fn(&TabId) -> Option<Value>) -> Option<Value> {
        let mut kids: Vec<Value> = s
            .children
            .iter()
            .filter_map(|c| match c {
                Node::Split(s) => self.snap_split(s, rec),
                Node::Pane(id) => self.pane(*id).ok().map(|p| {
                    leaf_value(
                        p.list.tabs.iter().filter_map(rec).collect(),
                        p.list.current.unwrap_or(0),
                        p.main,
                    )
                }),
            })
            .collect();
        match kids.len() {
            0 => None,
            1 => kids.pop(),
            _ => Some(map(vec![
                ("split", Value::Str(s.orientation.code().into())),
                (
                    "sizes",
                    Value::Seq(s.sizes.iter().map(|&x| Value::Int(x)).collect()),
                ),
                ("children", Value::Seq(kids)),
            ])),
        }
    }
}

/// Which dock zone a pointer at (`x`, `y`) hits in a pane of `w` × `h` (pane coordinates).
/// `None` outside the pane. First the guide cross: five squares of side `guide`, one in the middle
/// and one on each side of it (spacing `1.25 * guide`), shown only when the whole cross fits
/// inside the pane. Everywhere else the nearest edge decides, as in VS Code (the diagonals split
/// the pane into four sectors) and the middle third in both directions is `Center`.
pub fn hit_zone(w: f64, h: f64, x: f64, y: f64, guide: f64) -> Option<Dock> {
    if !(0.0..=w).contains(&x) || !(0.0..=h).contains(&y) || w <= 0.0 || h <= 0.0 {
        return None;
    }
    let (cx, cy, step, half) = (w / 2.0, h / 2.0, guide * 1.25, guide / 2.0);
    if cx - step - half >= 0.0 && cy - step - half >= 0.0 {
        let hit = |ox: f64, oy: f64| (x - cx - ox).abs() <= half && (y - cy - oy).abs() <= half;
        for (dock, ox, oy) in [
            (Dock::Center, 0.0, 0.0),
            (Dock::Left, -step, 0.0),
            (Dock::Right, step, 0.0),
            (Dock::Top, 0.0, -step),
            (Dock::Bottom, 0.0, step),
        ] {
            if hit(ox, oy) {
                return Some(dock);
            }
        }
    }
    let (nx, ny) = (x / w, y / h);
    if (1.0 / 3.0..=2.0 / 3.0).contains(&nx) && (1.0 / 3.0..=2.0 / 3.0).contains(&ny) {
        return Some(Dock::Center);
    }
    let edges = [
        (nx, Dock::Left),
        (1.0 - nx, Dock::Right),
        (ny, Dock::Top),
        (1.0 - ny, Dock::Bottom),
    ];
    edges
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, d)| d)
}

mod session;

pub use session::*;
