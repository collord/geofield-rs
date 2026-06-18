//! Surfaces as shared, referenced entities (`docs/stack_hierarchy_spec.md` §5).
//!
//! Today interfaces belong to their series. To represent **correlative conformity** — one
//! eroded surface that is simultaneously the erode-top of the stack BELOW and the
//! onlap-floor of the stack ABOVE — without storing it twice, surfaces are lifted to
//! referenced entities: a [`SurfaceStore`] holds each surface ONCE under a [`SurfaceId`],
//! and stack-element boundaries reference ids. A single surface referenced by two
//! boundaries is the DAG edge that makes the shared-surface case representable.
//!
//! Scope (v1): the DATA layer only. Sharing is exposed in the UI through the seam control,
//! NOT a general graph editor (design doc §5; a documented non-goal — Principle IV). The id
//! is a plain incrementing `u64` newtype rather than a `Uuid` — no external dependency is
//! warranted for an in-process, single-session store (Principle VI).

/// A stable handle to a shared surface entity. Opaque; allocated by [`SurfaceStore::insert`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SurfaceId(pub u64);

/// Which side of a referencing boundary a surface plays. The same [`SurfaceId`] can appear
/// as a [`Boundary::TruncationTop`] for the lower stack and a [`Boundary::LapAgainstFloor`]
/// for the upper stack — that pairing is exactly correlative conformity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Boundary {
    /// The erode/unconformity TOP a (lower) stack is truncated at.
    TruncationTop,
    /// The floor a (younger) onlapping stack laps against and pinches out on.
    LapAgainstFloor,
}

/// A reference from a stack element's boundary to a shared surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceRef {
    pub surface: SurfaceId,
    pub boundary: Boundary,
}

/// The shared-surface entity store: holds each surface payload ONCE, keyed by [`SurfaceId`],
/// and records which boundaries reference it. Generic over the surface payload `S` (e.g. a
/// fitted interface or its id within a model) so it does not dictate the geometry type.
pub struct SurfaceStore<S> {
    surfaces: Vec<S>,
    /// `refs[k]` = the boundaries referencing `SurfaceId(k)`. A surface shared by two
    /// boundaries has two entries; the surface payload is still stored only once.
    refs: Vec<Vec<Boundary>>,
}

impl<S> Default for SurfaceStore<S> {
    fn default() -> Self {
        Self { surfaces: Vec::new(), refs: Vec::new() }
    }
}

impl<S> SurfaceStore<S> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a surface once; return its id. The payload is NOT duplicated on later
    /// references — callers reference the returned id.
    pub fn insert(&mut self, surface: S) -> SurfaceId {
        let id = SurfaceId(self.surfaces.len() as u64);
        self.surfaces.push(surface);
        self.refs.push(Vec::new());
        id
    }

    /// Record that `boundary` references `id`. A surface may be referenced by two boundaries
    /// (truncation-top below + lap-against-floor above) — the correlative-conformity case —
    /// without copying the payload.
    pub fn reference(&mut self, id: SurfaceId, boundary: Boundary) -> SurfaceRef {
        if let Some(slot) = self.refs.get_mut(id.0 as usize) {
            slot.push(boundary);
        }
        SurfaceRef { surface: id, boundary }
    }

    /// Borrow the (single) stored payload for an id.
    pub fn get(&self, id: SurfaceId) -> Option<&S> {
        self.surfaces.get(id.0 as usize)
    }

    /// The boundaries currently referencing `id` (≥2 ⇒ a shared surface).
    pub fn references(&self, id: SurfaceId) -> &[Boundary] {
        self.refs.get(id.0 as usize).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// How many distinct surface payloads are stored (NOT counting references). The
    /// no-duplication invariant: sharing a surface across two boundaries does not grow this.
    pub fn len(&self) -> usize {
        self.surfaces.len()
    }

    pub fn is_empty(&self) -> bool {
        self.surfaces.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T048 — one surface referenced by TWO boundaries is stored once (no duplication) and
    /// both references resolve to the same id. This is the correlative-conformity DAG edge
    /// (FR-013).
    #[test]
    fn one_surface_two_boundaries_no_duplication() {
        let mut store: SurfaceStore<&str> = SurfaceStore::new();
        let unconformity = store.insert("the eroded surface");

        // The lower stack consumes it as its truncation top; the upper stack laps onto the
        // SAME surface. Two references, one stored payload.
        let lower = store.reference(unconformity, Boundary::TruncationTop);
        let upper = store.reference(unconformity, Boundary::LapAgainstFloor);

        assert_eq!(lower.surface, upper.surface, "both boundaries reference the same surface");
        assert_eq!(store.len(), 1, "the shared surface is stored ONCE, not duplicated");
        assert_eq!(
            store.references(unconformity),
            &[Boundary::TruncationTop, Boundary::LapAgainstFloor],
            "both boundary roles are recorded against the one surface"
        );
        assert_eq!(store.get(unconformity), Some(&"the eroded surface"));
    }

    /// Independent surfaces get distinct ids and each is stored once.
    #[test]
    fn distinct_surfaces_are_distinct_entities() {
        let mut store: SurfaceStore<u32> = SurfaceStore::new();
        let a = store.insert(10);
        let b = store.insert(20);
        assert_ne!(a, b);
        assert_eq!(store.len(), 2);
    }
}
