// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! A doubly linked list over storage the caller owns.
//!
//! The list holds no values. It holds a head, a tail, a length, and an
//! identifier; the links live in a slice of [`Link`] that the caller keeps
//! beside its own array of nodes, and a node is named by its index into
//! that slice. That is what a run queue over a fixed array of threads is,
//! and what an endpoint wait queue over the same array is, and neither
//! needs an allocator to be either (D-48).
//!
//! Several lists may run over one slice, which is the shape of one run
//! queue per priority. Each list carries an identifier, and a [`Link`]
//! records which list its node is in, so a node handed to the wrong list
//! is refused instead of being quietly stolen from the right one.

use crate::error::CollectionError;

/// The index that names no node: an empty head, an empty tail, and the
/// end of the list in either direction. It is also the identifier no list
/// may carry, because it is the mark of a node that is in none.
pub const NONE: u32 = u32::MAX;

/// The links of one node.
///
/// A caller keeps one of these per node, in a slice parallel to its own
/// array. The fields are private: a forged link would tear a list, and
/// nothing a caller can write into one is worth that.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Link {
    /// The node before this one, or [`NONE`].
    prev: u32,
    /// The node after this one, or [`NONE`].
    next: u32,
    /// The list this node is in, or [`NONE`] when it is in none.
    owner: u32,
}

impl Link {
    /// A node that is in no list.
    #[must_use]
    pub const fn new() -> Link {
        Link {
            prev: NONE,
            next: NONE,
            owner: NONE,
        }
    }

    /// The node before this one, or `None` at the head.
    #[must_use]
    pub const fn prev(&self) -> Option<u32> {
        if self.prev == NONE {
            None
        } else {
            Some(self.prev)
        }
    }

    /// The node after this one, or `None` at the tail.
    #[must_use]
    pub const fn next(&self) -> Option<u32> {
        if self.next == NONE {
            None
        } else {
            Some(self.next)
        }
    }

    /// The list this node is in, or `None` when it is in none.
    #[must_use]
    pub const fn owner(&self) -> Option<u32> {
        if self.owner == NONE {
            None
        } else {
            Some(self.owner)
        }
    }

    /// Whether the node is in some list.
    #[must_use]
    pub const fn is_linked(&self) -> bool {
        self.owner != NONE
    }
}

impl Default for Link {
    fn default() -> Link {
        Link::new()
    }
}

/// A doubly linked list over a caller's slice of [`Link`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IndexList {
    /// Which list this is, as recorded in the links of its nodes.
    id: u32,
    /// The first node, or [`NONE`].
    head: u32,
    /// The last node, or [`NONE`].
    tail: u32,
    /// How many nodes are in it.
    len: u32,
}

impl IndexList {
    /// An empty list identified by `id`.
    ///
    /// # Errors
    ///
    /// [`CollectionError::ListId`] when `id` is [`NONE`], which is the
    /// mark of a node in no list and therefore names no list.
    pub const fn new(id: u32) -> Result<IndexList, CollectionError> {
        if id == NONE {
            return Err(CollectionError::ListId);
        }
        Ok(IndexList {
            id,
            head: NONE,
            tail: NONE,
            len: 0,
        })
    }

    /// Which list this is.
    #[must_use]
    pub const fn id(&self) -> u32 {
        self.id
    }

    /// How many nodes are in it.
    #[must_use]
    pub const fn len(&self) -> u32 {
        self.len
    }

    /// Whether it holds no node.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The first node.
    #[must_use]
    pub const fn head(&self) -> Option<u32> {
        if self.head == NONE {
            None
        } else {
            Some(self.head)
        }
    }

    /// The last node.
    #[must_use]
    pub const fn tail(&self) -> Option<u32> {
        if self.tail == NONE {
            None
        } else {
            Some(self.tail)
        }
    }

    /// Puts `node` at the front.
    ///
    /// # Errors
    ///
    /// [`CollectionError::Index`] when `node` is outside `links`, and
    /// [`CollectionError::AlreadyLinked`] when it is in a list already.
    pub fn push_front(&mut self, links: &mut [Link], node: u32) -> Result<(), CollectionError> {
        Self::claim(links, node)?;
        let old_head = self.head;
        Self::write(links, node, NONE, old_head, self.id)?;
        if old_head == NONE {
            self.tail = node;
        } else {
            Self::set_prev(links, old_head, node)?;
        }
        self.head = node;
        self.len = self.len.wrapping_add(1);
        Ok(())
    }

    /// Puts `node` at the back.
    ///
    /// # Errors
    ///
    /// [`CollectionError::Index`] when `node` is outside `links`, and
    /// [`CollectionError::AlreadyLinked`] when it is in a list already.
    pub fn push_back(&mut self, links: &mut [Link], node: u32) -> Result<(), CollectionError> {
        Self::claim(links, node)?;
        let old_tail = self.tail;
        Self::write(links, node, old_tail, NONE, self.id)?;
        if old_tail == NONE {
            self.head = node;
        } else {
            Self::set_next(links, old_tail, node)?;
        }
        self.tail = node;
        self.len = self.len.wrapping_add(1);
        Ok(())
    }

    /// Puts `node` behind `after`, or at the front when `after` is `None`.
    ///
    /// This is the one operation that splices in the middle, and the list
    /// needs it because the `Link` fields are private: a caller that keeps
    /// its nodes in an order of its own cannot write them itself.
    ///
    /// # Errors
    ///
    /// [`CollectionError::Index`] when either node is outside `links`,
    /// [`CollectionError::AlreadyLinked`] when `node` is in a list already,
    /// and [`CollectionError::NotLinked`] when `after` is in another list
    /// or in none. A refused insert changes nothing.
    pub fn insert_after(
        &mut self,
        links: &mut [Link],
        node: u32,
        after: Option<u32>,
    ) -> Result<(), CollectionError> {
        let Some(after) = after else {
            return self.push_front(links, node);
        };
        Self::claim(links, node)?;
        let previous = *Self::at(links, after)?;
        if previous.owner != self.id {
            return Err(CollectionError::NotLinked(after));
        }
        let next = previous.next;
        if next == NONE {
            return self.push_back(links, node);
        }
        Self::write(links, node, after, next, self.id)?;
        Self::set_next(links, after, node)?;
        Self::set_prev(links, next, node)?;
        self.len = self.len.wrapping_add(1);
        Ok(())
    }

    /// Removes and returns the first node, or `None` when the list is
    /// empty.
    pub fn pop_front(&mut self, links: &mut [Link]) -> Option<u32> {
        let node = self.head()?;
        self.unlink(links, node).ok()?;
        Some(node)
    }

    /// Removes and returns the last node, or `None` when the list is
    /// empty.
    pub fn pop_back(&mut self, links: &mut [Link]) -> Option<u32> {
        let node = self.tail()?;
        self.unlink(links, node).ok()?;
        Some(node)
    }

    /// Takes `node` out of this list.
    ///
    /// # Errors
    ///
    /// [`CollectionError::Index`] when `node` is outside `links`, and
    /// [`CollectionError::NotLinked`] when it is in no list or in another
    /// one.
    pub fn unlink(&mut self, links: &mut [Link], node: u32) -> Result<(), CollectionError> {
        let link = *Self::at(links, node)?;
        if link.owner != self.id {
            return Err(CollectionError::NotLinked(node));
        }
        if link.prev == NONE {
            self.head = link.next;
        } else {
            Self::set_next(links, link.prev, link.next)?;
        }
        if link.next == NONE {
            self.tail = link.prev;
        } else {
            Self::set_prev(links, link.next, link.prev)?;
        }
        Self::write(links, node, NONE, NONE, NONE)?;
        self.len = self.len.wrapping_sub(1);
        Ok(())
    }

    /// Whether `node` is in this list.
    #[must_use]
    pub fn contains(&self, links: &[Link], node: u32) -> bool {
        Self::at(links, node).is_ok_and(|link| link.owner == self.id)
    }

    /// The nodes from the head to the tail.
    ///
    /// The walk stops after as many steps as the list says it is long, so
    /// a slice a caller has corrupted cannot make it run forever.
    #[must_use]
    pub const fn iter<'a>(&self, links: &'a [Link]) -> Iter<'a> {
        Iter {
            links,
            at: self.head,
            left: self.len,
        }
    }

    /// Checks that `node` names a link that is in no list, and reports why
    /// not when it does not.
    fn claim(links: &[Link], node: u32) -> Result<(), CollectionError> {
        let link = Self::at(links, node)?;
        if link.is_linked() {
            return Err(CollectionError::AlreadyLinked(node));
        }
        Ok(())
    }

    /// The link of `node`.
    fn at(links: &[Link], node: u32) -> Result<&Link, CollectionError> {
        let index = usize::try_from(node).map_err(|_| index_error(node))?;
        links.get(index).ok_or_else(|| index_error(node))
    }

    /// The link of `node`, to change.
    fn at_mut(links: &mut [Link], node: u32) -> Result<&mut Link, CollectionError> {
        let index = usize::try_from(node).map_err(|_| index_error(node))?;
        links.get_mut(index).ok_or_else(|| index_error(node))
    }

    /// Writes all three fields of a link.
    fn write(
        links: &mut [Link],
        node: u32,
        prev: u32,
        next: u32,
        owner: u32,
    ) -> Result<(), CollectionError> {
        let link = Self::at_mut(links, node)?;
        link.prev = prev;
        link.next = next;
        link.owner = owner;
        Ok(())
    }

    /// Points the predecessor of `node` at `prev`.
    fn set_prev(links: &mut [Link], node: u32, prev: u32) -> Result<(), CollectionError> {
        Self::at_mut(links, node)?.prev = prev;
        Ok(())
    }

    /// Points the successor of `node` at `next`.
    fn set_next(links: &mut [Link], node: u32, next: u32) -> Result<(), CollectionError> {
        Self::at_mut(links, node)?.next = next;
        Ok(())
    }
}

/// The error for a node that names no link in the slice.
fn index_error(node: u32) -> CollectionError {
    CollectionError::Index(usize::try_from(node).unwrap_or(usize::MAX))
}

/// The nodes of a list, from the head to the tail.
#[derive(Clone, Debug)]
pub struct Iter<'a> {
    /// The links the walk reads.
    links: &'a [Link],
    /// Where the walk stands, or [`NONE`] when it is done.
    at: u32,
    /// How many more nodes the list says there are.
    left: u32,
}

impl Iterator for Iter<'_> {
    type Item = u32;

    fn next(&mut self) -> Option<u32> {
        if self.left == 0 || self.at == NONE {
            return None;
        }
        let node = self.at;
        let link = IndexList::at(self.links, node).ok()?;
        self.at = link.next;
        self.left = self.left.wrapping_sub(1);
        Some(node)
    }
}
