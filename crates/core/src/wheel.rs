//! The wheel that opens where the finger pressed, and who sits in which sector.
//!
//! Two separate questions live here, and keeping them apart is the point:
//!
//! * [`Geometry`] is the ruler - where the hub ends, how wide a ring band is,
//!   and what a pointer at `(dx, dy)` from the wheel centre means. It depends
//!   only on the two numbers the user can dial (radius, dead zone).
//! * [`Dial`] is the seating plan - how the windows spread over the rings,
//!   which sector a pin lands on, and in what order the rest fill in. It
//!   depends only on how many windows there are and which directions are
//!   remembered.
//!
//! Neither knows what a window, a monitor or a mouse is. The shell measures
//! the cursor, hands us the offset from the press point, and reads back an
//! [`Aim`].
//!
//! Sector width is `360 / windows on that ring`, not a fixed 30 degrees. The
//! first draft of this feature used fixed sectors with empty ticks between
//! them, and on a desktop with six windows most of the ring was blank space
//! while the sectors themselves were too thin to hit while moving. Fat sectors
//! that share the circle out are what both reference implementations do, and
//! what the prototype in `docs/wheel-mock.html` settled on.

use crate::hit::Rect;

/// Windows per ring before the dial grows another one. Twelve is where a fixed
/// 30-degree sector lands anyway, so nothing below it is a compromise.
pub const SECTORS_PER_RING: usize = 12;
/// A ring is twelve sectors at most, so thirteen windows start a second ring and
/// the dial grows one ring at a time up to eight: 96 windows before any ring is
/// asked to hold more than the twelve that keep a name readable. The live wheel
/// shrinks the whole disc to fit the work area, so a tall stack of rings is still
/// wholly on screen.
pub const MAX_RINGS: usize = 8;
/// The seam between two rings, in logical pixels. Both references use 3.
pub const RING_GAP: f32 = 3.0;
/// An outer ring band is this fraction of the inner band. The inner band is
/// `radius - hub`, so outer rings stay visibly lighter than the one under them
/// instead of competing with it.
pub const OUTER_BAND_RATIO: f32 = 0.52;
/// Smallest and largest radius the slider offers, in logical pixels. At 110 the
/// inner ring's sectors are still a finger's width apart. The ceiling is only a
/// sanity bound for a hand-edited file: the live dial scales itself down to fit
/// the work area of the monitor it opens on, so a radius past the screen is
/// shown as large as the screen allows rather than clipped or refused.
pub const RADIUS_MIN: f32 = 110.0;
pub const RADIUS_MAX: f32 = 720.0;
pub const RADIUS_DEFAULT: f32 = 150.0;
/// How much of the radius is hub. Below 0.25 there is no room for the preview
/// text; above 0.65 the sectors are a thin crown and the drag from sector to
/// sector starts slipping through the gap.
pub const DEAD_ZONE_MIN: f32 = 0.25;
pub const DEAD_ZONE_MAX: f32 = 0.65;
pub const DEAD_ZONE_DEFAULT: f32 = 0.42;
/// Breathing room around the outermost ring - the wheel's own shadow, and the
/// margin that keeps a press near the screen edge from having to move the whole
/// wheel for one pixel of artwork. In logical pixels.
pub const PAD: f32 = 22.0;

/// One ring's radial span, in logical pixels from the wheel centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ring {
    pub lo: f32,
    pub hi: f32,
}

/// The ruler: radius and hub ratio, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub radius: f32,
    pub dead_zone: f32,
}

/// What a pointer at some offset from the centre means. Distance picks the ring,
/// angle picks the sector - the distance never picks the window, so reaching
/// further out does not change what you are pointing at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Aim {
    /// Inside the hub. Releasing here cancels.
    Hub,
    /// Between two rings, or in the seam outside the first one.
    Gap,
    /// Past the outermost ring.
    Past,
    /// A real sector that no window currently occupies.
    Empty { ring: usize, pos: usize },
    /// A sector holding a window.
    Slot { ring: usize, pos: usize, id: usize },
}

impl Geometry {
    /// Clamps the user's two numbers into the range the slider can produce, so
    /// a hand-edited settings file cannot make the wheel unusable.
    pub fn new(radius: f32, dead_zone: f32) -> Self {
        Geometry {
            radius: radius.clamp(RADIUS_MIN, RADIUS_MAX),
            dead_zone: dead_zone.clamp(DEAD_ZONE_MIN, DEAD_ZONE_MAX),
        }
    }

    /// The hub's edge - everything inside it is the preview, not a target.
    pub fn hub(&self) -> f32 {
        self.radius * self.dead_zone
    }

    /// An outer ring's thickness.
    pub fn band(&self) -> f32 {
        (self.radius - self.hub()) * OUTER_BAND_RATIO
    }

    /// How many rings `count` windows need.
    pub fn rings_for(count: usize) -> usize {
        count.div_ceil(SECTORS_PER_RING).clamp(1, MAX_RINGS)
    }

    /// The rings, innermost first, for a dial with that many rings. A ring count
    /// is driven by the window count, but a caller that already has a [`Dial`]
    /// passes its ring count here, where zero can only mean an empty desktop -
    /// one ring is still drawn for it, so there is a shape to aim at.
    pub fn ring_bounds(&self, rings: usize) -> Vec<Ring> {
        let rings = rings.clamp(1, MAX_RINGS);
        let mut out = vec![Ring {
            lo: self.hub(),
            hi: self.radius,
        }];
        let band = self.band();
        for r in 1..rings {
            let lo = self.radius + RING_GAP + (r - 1) as f32 * (band + RING_GAP);
            out.push(Ring { lo, hi: lo + band });
        }
        out
    }

    /// The full reach of the wheel including [`PAD`] - the half-size of the
    /// square the shell has to keep inside the work area.
    pub fn extent(&self, rings: usize) -> f32 {
        self.ring_bounds(rings)
            .last()
            .map(|b| b.hi + PAD)
            .unwrap_or(self.radius + PAD)
    }

    /// Reads a pointer offset from the wheel centre. `dx`/`dy` are in any unit,
    /// as long as both the offset and this geometry use the same one - the shell
    /// passes device pixels and scales the geometry by the cursor's monitor.
    pub fn aim(&self, dial: &Dial, dx: f32, dy: f32) -> Aim {
        let d = (dx * dx + dy * dy).sqrt();
        if d < self.hub() {
            return Aim::Hub;
        }
        // Bearings are measured clockwise from 12 o'clock, because that is the
        // direction the previous window always sits and the one thing the user
        // is asked to remember.
        let deg = (dy.atan2(dx).to_degrees() + 90.0).rem_euclid(360.0);
        let bounds = self.ring_bounds(dial.rings());
        let Some(ring) = bounds.iter().position(|b| d <= b.hi) else {
            return Aim::Past;
        };
        if d < bounds[ring].lo {
            return Aim::Gap;
        }
        let pos = dial.sector_at(ring, deg);
        match dial.slot(ring, pos).and_then(|s| s.occupant) {
            Some(id) => Aim::Slot { ring, pos, id },
            None => Aim::Empty { ring, pos },
        }
    }
}

/// The smallest clockwise gap between two bearings, in degrees.
pub fn ang_diff(a: f32, b: f32) -> f32 {
    let d = (a - b).abs() % 360.0;
    if d > 180.0 {
        360.0 - d
    } else {
        d
    }
}

/// One window offered on the wheel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Entry {
    /// The shell's handle for this window; the core only compares it.
    pub id: usize,
    /// The direction a pin remembers for this window, in degrees clockwise from
    /// 12 o'clock. `None` means the window is not pinned and gets seated by
    /// recency.
    pub pinned_at: Option<f32>,
    /// The ring the pin was dropped on, when the user aimed at one. `None` is a
    /// direction-only pin: it takes the nearest free sector anywhere, as pins
    /// written before rings were stored always did.
    pub pinned_ring: Option<usize>,
    /// How much of its ring this window's sector claims, relative to its
    /// neighbours. `None` is one, so a caller that does not care about sizes
    /// keeps the equal sectors the dial has always drawn.
    pub pinned_weight: Option<f32>,
    /// The place this window holds on the user's favourites list, counting from
    /// zero. `None` means it is not one of them.
    ///
    /// It is a rank and not a flag because the list is in the order the windows
    /// were added, while `entries` arrive in most-recently-used order: a flag would
    /// seat the favourites by recency, and the whole point of a list the user
    /// builds by hand is that adding something new does not move what is already
    /// on it.
    ///
    /// A favourite that is also pinned obeys the pin, because the direction is the
    /// part the user aimed at. The favourite's promise is "the crowd never pushes
    /// this window outwards", and a pin pointing at the outer ring is not the crowd.
    pub favourite_at: Option<usize>,
}

/// One sector of the wheel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub ring: usize,
    pub pos: usize,
    /// The centre of this sector, degrees clockwise from 12 o'clock.
    pub bearing: f32,
    /// The angular width of this sector. Equal sectors are `360 / ring count`;
    /// a pinned window that asked for a larger share widens its own and takes
    /// the room from the rest of the ring.
    pub span: f32,
    pub occupant: Option<usize>,
}

/// The seating plan for one opening of the wheel.
#[derive(Clone, Debug, PartialEq)]
pub struct Dial {
    slots: Vec<Slot>,
    ring_counts: Vec<usize>,
}

impl Dial {
    /// Seats `entries`, which the caller must pass in most-recently-used order
    /// (most recent first). Three claims, in this order: a pin takes the free
    /// sector nearest the direction it remembers, a favourite takes a place on
    /// the inner ring before anything undirected does, and the rest fill the
    /// sectors that are left, innermost ring first, clockwise from 12. The
    /// favourite's claim is only exercised when the inner ring cannot hold
    /// everybody; while it can, `entries`' own order is the arrangement.
    ///
    /// A pin is a direction and never a sector number, which is what keeps
    /// "my mail is at 6 o'clock" true while the window count changes underneath
    /// it. When two pins want the same direction the later one takes the next
    /// nearest free sector, and the pin mark on the artwork is what tells the
    /// user that happened.
    ///
    /// A pin beats a favourite when one window has both, and it has to: the
    /// favourite's promise is "this window is never pushed outwards by the
    /// crowd", and a direction the user aimed at and released is not the crowd.
    /// A pin that points at the outer ring is the user asking for the outer ring.
    pub fn build(entries: &[Entry]) -> Dial {
        let n = entries.len();
        if n == 0 {
            return Dial {
                slots: Vec::new(),
                ring_counts: Vec::new(),
            };
        }
        let rings = Geometry::rings_for(n);
        let base = n / rings;
        let wider = n - rings * base;
        let ring_counts: Vec<usize> = (0..rings).map(|r| base + usize::from(r < wider)).collect();

        let mut slots: Vec<Slot> = Vec::with_capacity(n);
        for (ring, &count) in ring_counts.iter().enumerate() {
            for pos in 0..count {
                slots.push(Slot {
                    ring,
                    pos,
                    bearing: (360.0 * pos as f32 / count as f32).rem_euclid(360.0),
                    span: 360.0 / count as f32,
                    occupant: None,
                });
            }
        }

        for e in entries
            .iter()
            .filter_map(|e| e.pinned_at.map(|b| (e.id, b, e.pinned_ring)))
        {
            let (id, bearing, ring) = e;
            let mut best: Option<usize> = None;
            let mut nearest = f32::INFINITY;
            // A pin that remembers a ring is the user asking for that ring, so
            // the search starts narrow. Only when the ring has no free sector
            // left does the pin fall back to the whole dial: a pin that vanishes
            // because a ring filled up would be a pin nobody trusts.
            let search_ring = ring.filter(|r| *r < rings);
            for pass in [search_ring, None] {
                for (i, s) in slots.iter().enumerate() {
                    if s.occupant.is_some() {
                        continue;
                    }
                    if let Some(ring) = pass {
                        if s.ring != ring {
                            continue;
                        }
                    }
                    let d = ang_diff(s.bearing, bearing);
                    if d < nearest {
                        nearest = d;
                        best = Some(i);
                    }
                }
                if best.is_some() {
                    break;
                }
            }
            if let Some(i) = best {
                slots[i].occupant = Some(id);
            }
        }

        // The favourites claim the inner ring before the crowd does, and they
        // spill outwards only when the ring has no sector left - "on the wheel"
        // is the stronger half of the promise. That reservation is only worth
        // what it costs when there is a crowd: while every window still fits on
        // the inner ring, the remembered plan alone decides who sits where.
        // Seating the favourites first in that case let their list rank outrank
        // the plan, so a drag between a favourite and an ordinary window could
        // not be written down - the builder kept putting the favourite back in
        // front of it, and the sector never moved.
        let unpinned: Vec<&Entry> = entries.iter().filter(|e| e.pinned_at.is_none()).collect();
        let inner_free = slots
            .iter()
            .filter(|s| s.ring == 0 && s.occupant.is_none())
            .count();
        if unpinned.len() <= inner_free {
            for e in unpinned {
                if let Some(i) = slots.iter().position(|s| s.occupant.is_none()) {
                    slots[i].occupant = Some(e.id);
                }
            }
        } else {
            // The favourites, oldest first, each on the inner ring while the
            // inner ring has a sector left. Sorted by the rank rather than taken
            // in list order because the list arrives in recency order; when they
            // run out of inner ring they spill outwards in the same order rather
            // than dropping off the dial.
            let mut preferred: Vec<(usize, usize)> = unpinned
                .iter()
                .filter_map(|e| e.favourite_at.map(|rank| (rank, e.id)))
                .collect();
            preferred.sort_by_key(|(rank, _)| *rank);
            for (_, id) in preferred {
                let inner = slots
                    .iter()
                    .position(|s| s.ring == 0 && s.occupant.is_none());
                if let Some(i) = inner.or_else(|| slots.iter().position(|s| s.occupant.is_none())) {
                    slots[i].occupant = Some(id);
                }
            }

            let mut free = slots.iter_mut().filter(|s| s.occupant.is_none());
            for e in unpinned.iter().filter(|e| e.favourite_at.is_none()) {
                if let Some(s) = free.next() {
                    s.occupant = Some(e.id);
                }
            }
        }

        // Sizes are a second pass over the plan: the seats keep the order they
        // were given, and each ring shares its 360 degrees out in proportion to
        // the weights. A ring where every window asks for the same room is left
        // exactly as it was, so nothing moves unless the user resized something.
        for ring in 0..rings {
            let occupants: Vec<(usize, usize, f32)> = slots
                .iter()
                .filter(|s| s.ring == ring)
                .filter_map(|s| {
                    let id = s.occupant?;
                    let weight = entries
                        .iter()
                        .find(|e| e.id == id)
                        .and_then(|e| e.pinned_weight)
                        .unwrap_or(1.0)
                        .max(0.01);
                    Some((s.pos, id, weight))
                })
                .collect();
            let weighted = occupants
                .iter()
                .any(|(_, _, w)| (w - 1.0).abs() > 0.001);
            if !weighted {
                continue;
            }
            let total: f32 = occupants.iter().map(|(_, _, w)| *w).sum();
            if total <= 0.0 {
                continue;
            }
            let mut start = 0.0f32;
            for (pos, _, weight) in occupants {
                let span = 360.0 * weight / total;
                if let Some(slot) = slots.iter_mut().find(|s| s.ring == ring && s.pos == pos) {
                    slot.span = span;
                    slot.bearing = (start + span / 2.0).rem_euclid(360.0);
                }
                start += span;
            }
        }

        Dial { slots, ring_counts }
    }

    /// How many rings this dial is drawn on.
    pub fn rings(&self) -> usize {
        self.ring_counts.len()
    }

    /// Sectors on one ring.
    pub fn ring_count(&self, ring: usize) -> usize {
        self.ring_counts.get(ring).copied().unwrap_or(0)
    }

    /// The angular width of one sector on a ring, in degrees.
    pub fn sector_width(&self, ring: usize) -> f32 {
        let count = self.ring_count(ring);
        if count == 0 {
            360.0
        } else {
            360.0 / count as f32
        }
    }

    /// Which sector of a ring a bearing falls in. Sectors are centred on their
    /// bearing, so the seam between two is halfway between them; a resized
    /// sector is wider and owns the extra room on both sides.
    pub fn sector_at(&self, ring: usize, deg: f32) -> usize {
        let count = self.ring_count(ring);
        if count == 0 {
            return 0;
        }
        for slot in self.slots.iter().filter(|s| s.ring == ring) {
            let start = (slot.bearing - slot.span / 2.0).rem_euclid(360.0);
            if (deg - start).rem_euclid(360.0) < slot.span {
                return slot.pos;
            }
        }
        // Only reachable through rounding at a seam: take the nearest centre.
        let span = 360.0 / count as f32;
        (((deg + span / 2.0) / span).floor() as i64).rem_euclid(count as i64) as usize
    }

    pub fn slot(&self, ring: usize, pos: usize) -> Option<&Slot> {
        self.slots.iter().find(|s| s.ring == ring && s.pos == pos)
    }

    /// Where a window is sitting right now, if it is on the wheel at all.
    pub fn find(&self, id: usize) -> Option<&Slot> {
        self.slots.iter().find(|s| s.occupant == Some(id))
    }

    pub fn slots(&self) -> &[Slot] {
        &self.slots
    }

    pub fn ring_counts(&self) -> &[usize] {
        &self.ring_counts
    }
}

/// Keeps the whole wheel inside `area` instead of clipping it, and reports
/// whether it had to move. A press near the edge opens the wheel shifted, so
/// the sectors the user is aiming at stay complete - cropping the dial was the
/// one thing both references disagreed about and the one that reads worse.
pub fn wheel_centre(x: f32, y: f32, extent: f32, area: Rect) -> (f32, f32, bool) {
    let w = (area.w / 2.0 - extent).max(0.0);
    let h = (area.h / 2.0 - extent).max(0.0);
    let (mid_x, mid_y) = area.center();
    let cx = mid_x - w + (x - (mid_x - w)).clamp(0.0, 2.0 * w);
    let cy = mid_y - h + (y - (mid_y - h)).clamp(0.0, 2.0 * h);
    (cx, cy, cx != x || cy != y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(ids: &[usize]) -> Vec<Entry> {
        ids.iter()
            .map(|&id| Entry {
                id,
                pinned_at: None,
                pinned_ring: None,
                pinned_weight: None,
                favourite_at: None,
            })
            .collect()
    }

    #[test]
    fn a_bigger_sector_takes_room_from_the_rest_of_its_ring_and_leaves_no_gap() {
        let entries: Vec<Entry> = (0..6)
            .map(|id| Entry {
                id,
                pinned_at: if id == 2 { Some(120.0) } else { None },
                pinned_ring: None,
                pinned_weight: if id == 2 { Some(2.0) } else { None },
                favourite_at: None,
            })
            .collect();
        let d = Dial::build(&entries);
        // One double-weight sector and five single ones: 360 / 7 each, so 2/7
        // of the ring is the big one and nothing is left over.
        let big = d.find(2).unwrap().span;
        let rest = d.find(0).unwrap().span;
        assert!((big - 720.0 / 7.0).abs() < 1e-3, "{big}");
        assert!((rest - 360.0 / 7.0).abs() < 1e-3, "{rest}");
        let total: f32 = d.slots().iter().map(|s| s.span).sum();
        assert!((total - 360.0).abs() < 1e-3, "{total}");
        // The sectors meet edge to edge, starting at twelve o'clock.
        let mut edge = 0.0f32;
        for pos in 0..6 {
            let slot = d.slot(0, pos).unwrap();
            assert!(
                (slot.bearing - slot.span / 2.0 - edge).abs() < 1e-3,
                "pos {pos}"
            );
            edge += slot.span;
        }
        assert!((edge - 360.0).abs() < 1e-3);
        // A point inside the widened sector is still it, not its neighbour.
        assert_eq!(d.sector_at(0, 150.0), 2);
    }

    #[test]
    fn equal_weights_leave_the_dial_exactly_where_it_was() {
        let ids: Vec<usize> = (0..12).collect();
        let d = Dial::build(&plain(&ids));
        for slot in d.slots() {
            assert!((slot.span - 30.0).abs() < 1e-3);
        }
        assert_eq!(d.find(0).unwrap().bearing, 0.0);
        assert_eq!(d.find(1).unwrap().bearing, 30.0);
    }

    #[test]
    fn sectors_share_the_circle_so_six_windows_are_sixty_degrees_each() {
        let d = Dial::build(&plain(&(0..6).collect::<Vec<_>>()));
        assert_eq!(d.rings(), 1);
        assert_eq!(d.ring_count(0), 6);
        assert!((d.sector_width(0) - 60.0).abs() < 1e-4);
        // Twelve is where the old fixed 30 degrees would have started, and
        // thirteen is where a second ring begins rather than a thinner sector.
        let d = Dial::build(&plain(&(0..12).collect::<Vec<_>>()));
        assert_eq!(d.rings(), 1);
        assert!((d.sector_width(0) - 30.0).abs() < 1e-4);
        let d = Dial::build(&plain(&(0..13).collect::<Vec<_>>()));
        assert_eq!(d.rings(), 2);
        assert_eq!(d.ring_counts(), &[7, 6]);
    }

    #[test]
    fn the_windows_spread_evenly_and_the_rings_never_get_skinny() {
        for n in 1..=40usize {
            let ids: Vec<usize> = (0..n).collect();
            let d = Dial::build(&plain(&ids));
            assert_eq!(d.slots().len(), n, "every window gets a sector at {n}");
            let widest = (0..d.rings())
                .map(|r| d.sector_width(r))
                .fold(0.0f32, f32::max);
            let narrowest = (0..d.rings())
                .map(|r| d.sector_width(r))
                .fold(360.0f32, f32::min);
            // Two rings holding the same count differ by at most one sector, so
            // the dial never shows a fat ring next to a crowded one.
            assert!(widest / narrowest <= 1.2, "{n}: {narrowest}..{widest}");
            if n > 1 {
                assert!(narrowest >= 25.0, "{n}: narrowest {narrowest}");
            }
        }
    }

    #[test]
    fn twelve_oclock_is_the_previous_window_and_the_rings_start_there_too() {
        let ids: Vec<usize> = (0..14).collect();
        let d = Dial::build(&plain(&ids));
        assert_eq!(d.rings(), 2);
        assert_eq!(d.find(0).unwrap().bearing, 0.0);
        assert_eq!(d.find(0).unwrap().ring, 0);
        assert_eq!(d.find(0).unwrap().pos, 0);
        // Recency runs clockwise, then continues on the next ring from 12 again.
        assert_eq!(d.find(1).unwrap().pos, 1);
        assert_eq!(d.find(6).unwrap().pos, 6);
        assert_eq!(d.slot(1, 0).unwrap().bearing, 0.0);
        assert_eq!(d.slot(1, 0).unwrap().occupant, Some(7));
        assert!((d.slot(1, 1).unwrap().bearing - 360.0 / 7.0).abs() < 1e-4);
    }

    #[test]
    fn a_pin_keeps_its_direction_while_the_count_changes() {
        let with = |ids: &[usize], pin: usize, at: f32| -> Vec<Entry> {
            ids.iter()
                .map(|&id| Entry {
                    id,
                    pinned_at: if id == pin { Some(at) } else { None },
                    pinned_ring: None,
                    pinned_weight: None,
                    favourite_at: None,
                })
                .collect()
        };
        // Six windows: 60-degree sectors, and 180 is a sector centre.
        let d = Dial::build(&with(&[0, 1, 2, 3, 4, 5], 3, 180.0));
        assert_eq!(d.find(3).unwrap().bearing, 180.0);
        // Twelve windows: the sectors are half as wide, and the pin still sits
        // due south. This is the whole reason a pin is a direction.
        let d = Dial::build(&with(&(0..12).collect::<Vec<_>>(), 3, 180.0));
        assert_eq!(d.find(3).unwrap().bearing, 180.0);
        // Thirteen, so the pin crosses onto a second ring: still nearest 180,
        // whichever ring has a sector there.
        let d = Dial::build(&with(&(0..13).collect::<Vec<_>>(), 3, 180.0));
        assert!(ang_diff(d.find(3).unwrap().bearing, 180.0) <= 30.0);
        // A pin that is the only thing keeping a window on the wheel still gets
        // seated - the caller filters the current window out, except for pins.
        let d = Dial::build(&with(&[9], 9, 300.0));
        assert_eq!(d.find(9).unwrap().bearing, 0.0);
    }

    #[test]
    fn a_pin_that_remembers_a_ring_stays_on_that_ring() {
        let rings = |ring: Option<usize>| -> Vec<Entry> {
            (0..13)
                .map(|id| Entry {
                    id,
                    pinned_at: if id == 3 { Some(180.0) } else { None },
                    pinned_ring: if id == 3 { ring } else { None },
                    pinned_weight: None,
                    favourite_at: None,
                })
                .collect()
        };
        // Thirteen windows give the dial two rings, so the claim is observable.
        let d = Dial::build(&rings(Some(0)));
        assert_eq!(d.find(3).unwrap().ring, 0);
        assert!(ang_diff(d.find(3).unwrap().bearing, 180.0) <= 30.0);
        // The direction-only form is the file every earlier version wrote: it must
        // still land on the nearest sector anywhere, here the outer ring's 180.
        let d = Dial::build(&rings(None));
        assert_eq!(d.find(3).unwrap().ring, 1);

        // A ring that is already full cannot keep the pin there; it falls back
        // outwards rather than dropping the window off the dial.
        let full: Vec<Entry> = (0..13)
            .map(|id| Entry {
                id,
                pinned_at: (id <= 6).then_some(id as f32 * 60.0),
                pinned_ring: (id <= 6).then_some(1),
                pinned_weight: None,
                favourite_at: None,
            })
            .collect();
        let d = Dial::build(&full);
        for id in 0..6 {
            assert_eq!(d.find(id).unwrap().ring, 1, "pin {id} kept its ring");
        }
        assert_eq!(d.find(6).unwrap().ring, 0, "the extra pin fell back");
    }

    #[test]
    fn two_pins_on_one_direction_move_to_the_next_free_sector() {
        let entries = vec![
            Entry {
                id: 1,
                pinned_at: Some(0.0),
                pinned_ring: None,
                pinned_weight: None,
                favourite_at: None,
            },
            Entry {
                id: 2,
                pinned_at: Some(10.0),
                pinned_ring: None,
                pinned_weight: None,
                favourite_at: None,
            },
            Entry {
                id: 3,
                pinned_at: None,
                pinned_ring: None,
                pinned_weight: None,
                favourite_at: None,
            },
        ];
        let d = Dial::build(&entries);
        assert_eq!(d.find(1).unwrap().bearing, 0.0);
        // The first pin took 12 o'clock, so the second takes the next nearest.
        assert_eq!(d.find(2).unwrap().bearing, 120.0);
        assert_eq!(d.find(3).unwrap().bearing, 240.0);
        assert_eq!(d.slots().len(), 3);
    }

    /// Windows seated by recency, with `favs` marked as favourites in the order
    /// the user added them.
    fn seated(ids: &[usize], favs: &[usize]) -> Vec<Entry> {
        ids.iter()
            .map(|&id| Entry {
                id,
                pinned_at: None,
                pinned_ring: None,
                pinned_weight: None,
                favourite_at: favs.iter().position(|f| *f == id),
            })
            .collect()
    }

    #[test]
    fn a_favourite_is_seated_on_the_inner_ring_before_the_crowd() {
        // Thirteen windows, so the inner ring has seven sectors and the crowd has
        // to start spilling. The two favourites are the *least* recent windows, and
        // they still get the first two sectors of the inner ring.
        let ids: Vec<usize> = (0..13).collect();
        let d = Dial::build(&seated(&ids, &[11, 12]));
        assert_eq!(d.ring_counts(), &[7, 6]);
        assert_eq!(d.find(11).map(|s| (s.ring, s.pos)), Some((0, 0)));
        assert_eq!(d.find(12).map(|s| (s.ring, s.pos)), Some((0, 1)));
        // Recency fills behind them rather than in front of them, so the window the
        // user just left moves two sectors off twelve, not off the dial.
        assert_eq!(d.find(0).map(|s| (s.ring, s.pos)), Some((0, 2)));
        assert_eq!(d.find(1).map(|s| (s.ring, s.pos)), Some((0, 3)));
        assert_eq!(d.find(4).map(|s| (s.ring, s.pos)), Some((0, 6)));
        assert_eq!(d.find(5).map(|s| (s.ring, s.pos)), Some((1, 0)));
    }

    #[test]
    fn while_everything_fits_on_the_inner_ring_the_plan_decides_where_a_favourite_sits() {
        // Four windows and four sectors: there is no crowd for a favourite to be
        // protected from, so the remembered plan is the arrangement even when the
        // favourites list names `3` first. A drag between a favourite and an
        // ordinary window has to be writable down, and only the plan can say it.
        let d = Dial::build(&seated(&[0, 1, 2, 3], &[3, 1]));
        assert_eq!(d.find(0).map(|s| (s.ring, s.pos)), Some((0, 0)));
        assert_eq!(d.find(1).map(|s| (s.ring, s.pos)), Some((0, 1)));
        assert_eq!(d.find(2).map(|s| (s.ring, s.pos)), Some((0, 2)));
        assert_eq!(d.find(3).map(|s| (s.ring, s.pos)), Some((0, 3)));
        // The same plan still seats the same way frame after frame; only the
        // crowded case below leans on the rank.
        let d = Dial::build(&seated(&[0, 1, 2, 3], &[1, 3]));
        assert_eq!(d.find(3).map(|s| (s.ring, s.pos)), Some((0, 3)));
    }

    #[test]
    fn too_many_favourites_spill_outwards_in_their_own_order() {
        // Fifteen favourites, added in the order they are listed: the inner ring
        // takes the first eight and the next seven go outward, still in that order.
        let ids: Vec<usize> = (0..15).collect();
        let d = Dial::build(&seated(&ids, &ids));
        assert_eq!(d.ring_counts(), &[8, 7]);
        for id in 0..8usize {
            assert_eq!(d.find(id).map(|s| (s.ring, s.pos)), Some((0, id)));
        }
        for id in 8..15usize {
            assert_eq!(d.find(id).map(|s| (s.ring, s.pos)), Some((1, id - 8)));
        }
    }

    #[test]
    fn a_pinned_favourite_goes_where_it_was_pointed() {
        // `11` is a favourite and pinned at the bearing that is the sixth sector of
        // the inner ring. The pin is served first, so it takes that sector and the
        // other favourite moves up to twelve o'clock - the direction wins over the
        // list, which is what the pin mark on the artwork is there to say.
        let entries: Vec<Entry> = (0..13)
            .map(|id| Entry {
                id,
                pinned_at: if id == 11 { Some(257.14) } else { None },
                pinned_ring: None,
                pinned_weight: None,
                favourite_at: [11usize, 12].iter().position(|f| *f == id),
            })
            .collect();
        let d = Dial::build(&entries);
        let seated = d.find(11).unwrap();
        assert_eq!((seated.ring, seated.pos), (0, 5));
        assert!(ang_diff(seated.bearing, 257.14) < 1.0);
        assert_eq!(d.find(12).map(|s| (s.ring, s.pos)), Some((0, 0)));
    }

    #[test]
    fn distance_picks_the_ring_and_angle_picks_the_sector() {
        let g = Geometry::new(150.0, 0.42);
        let d = Dial::build(&plain(&(0..13).collect::<Vec<_>>()));
        let hub = g.hub();
        // Just outside the hub, at 60 degrees: first ring, second sector.
        let aim = |deg: f32, r: f32| {
            let a = (deg - 90.0).to_radians();
            g.aim(&d, a.cos() * r, a.sin() * r)
        };
        assert_eq!(
            aim(0.0, hub + 1.0),
            Aim::Slot {
                ring: 0,
                pos: 0,
                id: 0
            }
        );
        assert_eq!(
            aim(60.0, hub + 1.0),
            Aim::Slot {
                ring: 0,
                pos: 1,
                id: 1
            }
        );
        // The same angle, further out, is still the same window on the first
        // ring until the pointer crosses the seam.
        assert_eq!(
            aim(60.0, 149.0),
            Aim::Slot {
                ring: 0,
                pos: 1,
                id: 1
            }
        );
        // In the seam, and past the last ring, nothing is picked.
        assert_eq!(aim(60.0, 151.0), Aim::Gap);
        assert_eq!(aim(60.0, 300.0), Aim::Past);
        // Inside the hub is always the hub, whatever the angle.
        assert_eq!(aim(275.0, hub - 1.0), Aim::Hub);
    }

    #[test]
    fn the_seam_at_twelve_and_the_sector_edges_round_the_right_way() {
        let g = Geometry::new(150.0, 0.42);
        let d = Dial::build(&plain(&(0..6).collect::<Vec<_>>()));
        let r = g.hub() + 30.0;
        let aim = |deg: f32| {
            let a = (deg - 90.0).to_radians();
            g.aim(&d, a.cos() * r, a.sin() * r)
        };
        // Sectors are centred on their bearing, so 359.9 is still 12 o'clock's
        // sector and 30 is the seam between the first two.
        assert_eq!(
            aim(359.9),
            Aim::Slot {
                ring: 0,
                pos: 0,
                id: 0
            }
        );
        assert_eq!(
            aim(29.9),
            Aim::Slot {
                ring: 0,
                pos: 0,
                id: 0
            }
        );
        assert_eq!(
            aim(30.1),
            Aim::Slot {
                ring: 0,
                pos: 1,
                id: 1
            }
        );
        assert_eq!(
            aim(180.0),
            Aim::Slot {
                ring: 0,
                pos: 3,
                id: 3
            }
        );
    }

    #[test]
    fn an_empty_desktop_and_a_huge_one_both_stay_inside_the_ring_ceiling() {
        assert_eq!(Dial::build(&[]).slots().len(), 0);
        assert_eq!(Dial::build(&[]).rings(), 0);
        // The geometry still draws one ring for nothing, so the shell has a
        // shape to aim at instead of a divide-by-zero.
        let g = Geometry::new(150.0, 0.42);
        assert_eq!(g.ring_bounds(0).len(), 1);
        assert_eq!(
            g.ring_bounds(0)[0],
            Ring {
                lo: g.hub(),
                hi: 150.0
            }
        );
        assert_eq!(g.ring_bounds(9).len(), MAX_RINGS);
        let d = Dial::build(&plain(&(0..60).collect::<Vec<_>>()));
        assert_eq!(d.rings(), 5);
        assert_eq!(d.slots().len(), 60);
        let huge = Dial::build(&plain(&(0..120).collect::<Vec<_>>()));
        assert_eq!(huge.rings(), MAX_RINGS);
        // Aim at a sector the seating plan never had room for.
        assert!(matches!(
            g.aim(&d, 0.0, -(g.hub() + 1.0)),
            Aim::Slot { pos: 0, .. }
        ));
    }

    #[test]
    fn the_ring_bands_stack_outwards_with_a_seam_between_them() {
        let g = Geometry::new(150.0, 0.42);
        let b = g.ring_bounds(3);
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].hi, 150.0);
        assert_eq!(b[1].lo, 150.0 + RING_GAP);
        assert!((b[1].hi - b[1].lo - g.band()).abs() < 1e-4);
        assert_eq!(b[2].lo, b[1].hi + RING_GAP);
        // Nothing overlaps, so a pointer can only ever be in one band.
        for w in b.windows(2) {
            assert!(w[0].hi < w[1].lo);
        }
        assert!((g.extent(3) - (b[2].hi + PAD)).abs() < 1e-4);
    }

    #[test]
    fn the_wheel_moves_whole_rather_than_getting_cut() {
        let g = Geometry::new(150.0, 0.42);
        let e = g.extent(1);
        let desk = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        // A press in the middle keeps the wheel where the finger is.
        assert_eq!(wheel_centre(960.0, 540.0, e, desk), (960.0, 540.0, false));
        // A press near the left edge pushes the whole dial right, by exactly as
        // much as the wheel needs, and says it moved.
        let (cx, cy, moved) = wheel_centre(20.0, 540.0, e, desk);
        assert!(moved);
        assert_eq!(cx, e);
        assert_eq!(cy, 540.0);
        // Near a corner it moves on both axes and still fits.
        let (cx, cy, moved) = wheel_centre(1900.0, 1070.0, e, desk);
        assert!(moved);
        assert!(cx + e <= 1920.0 && cy + e <= 1080.0);
        // A wheel bigger than the area is not allowed to flip sides: it stays
        // where it was rather than jittering between two clamps.
        let small = Rect::new(0.0, 0.0, 100.0, 100.0);
        assert_eq!(wheel_centre(50.0, 50.0, 200.0, small), (50.0, 50.0, false));
    }

    #[test]
    fn the_user_numbers_get_clamped_not_trusted() {
        let g = Geometry::new(10.0, 0.9);
        assert_eq!(g.radius, RADIUS_MIN);
        assert_eq!(g.dead_zone, DEAD_ZONE_MAX);
        let g = Geometry::new(4000.0, 0.05);
        assert_eq!(g.radius, RADIUS_MAX);
        assert_eq!(g.dead_zone, DEAD_ZONE_MIN);
        // The hub is always smaller than the ring it sits inside, so aiming can
        // never report a sector and the hub for the same pointer.
        assert!(g.hub() < g.radius);
    }
}
