// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

//! The addresses the stack carries: a hardware address, an IPv4 address,
//! an IPv4 network, and a transport port.
//!
//! Each is a value over its bytes and not a string. The text form is the
//! canonical one in both directions: what [`Display`](core::fmt::Display)
//! writes is what `parse` reads, and every other spelling is an error.
//! `010.0.0.1` is therefore refused rather than read as octal, which is
//! the reading that lets one address be written two ways and compared
//! once.

use core::fmt;

use crate::error::WireError;

/// A 48-bit Ethernet address.
///
/// The text form is six lower-case hex pairs joined by colons. Reading
/// takes either case, because a MAC address is written by hand as often as
/// by a program; writing is lower case, because a value has one spelling.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MacAddr([u8; 6]);

impl MacAddr {
    /// How many bytes the address occupies on the wire.
    pub const LEN: usize = 6;

    /// `ff:ff:ff:ff:ff:ff`, the address every station receives.
    pub const BROADCAST: MacAddr = MacAddr([0xFF; 6]);

    /// `00:00:00:00:00:00`, the address a station has before it knows one.
    pub const UNSPECIFIED: MacAddr = MacAddr([0x00; 6]);

    /// The address of these six bytes, in wire order.
    #[must_use]
    pub const fn new(octets: [u8; 6]) -> MacAddr {
        MacAddr(octets)
    }

    /// The six bytes, in wire order.
    #[must_use]
    pub const fn octets(self) -> [u8; 6] {
        self.0
    }

    /// Whether this is the broadcast address.
    #[must_use]
    pub fn is_broadcast(self) -> bool {
        self == MacAddr::BROADCAST
    }

    /// Whether this is the all-zero address.
    #[must_use]
    pub fn is_unspecified(self) -> bool {
        self == MacAddr::UNSPECIFIED
    }

    /// Whether the group bit is set. The broadcast address is a multicast
    /// address by this test, which is what the hardware does with it.
    #[must_use]
    pub const fn is_multicast(self) -> bool {
        let [first, ..] = self.0;
        first & 0x01 != 0
    }

    /// Whether the address names one station.
    #[must_use]
    pub const fn is_unicast(self) -> bool {
        !self.is_multicast()
    }

    /// The address written as six hex pairs joined by colons.
    ///
    /// # Errors
    ///
    /// [`WireError::Address`] when the text is not six groups of exactly
    /// two hex digits.
    pub fn parse(text: &str) -> Result<MacAddr, WireError> {
        let mut octets = [0u8; 6];
        let mut groups = text.split(':');
        for slot in &mut octets {
            let group = groups.next().ok_or(WireError::Address)?;
            let mut digits = group.bytes();
            let high = hex_digit(digits.next().ok_or(WireError::Address)?)?;
            let low = hex_digit(digits.next().ok_or(WireError::Address)?)?;
            if digits.next().is_some() {
                return Err(WireError::Address);
            }
            // Both digits are below sixteen, so the shift keeps every bit.
            *slot = high.wrapping_shl(4) | low;
        }
        if groups.next().is_some() {
            return Err(WireError::Address);
        }
        Ok(MacAddr(octets))
    }
}

impl fmt::Display for MacAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for octet in self.0 {
            if !first {
                f.write_str(":")?;
            }
            first = false;
            write!(f, "{octet:02x}")?;
        }
        Ok(())
    }
}

/// One hex digit as its value, either case.
fn hex_digit(byte: u8) -> Result<u8, WireError> {
    let value = match byte {
        b'0'..=b'9' => byte.checked_sub(b'0'),
        b'a'..=b'f' => byte.checked_sub(b'a').and_then(|d| d.checked_add(10)),
        b'A'..=b'F' => byte.checked_sub(b'A').and_then(|d| d.checked_add(10)),
        _ => None,
    };
    value.ok_or(WireError::Address)
}

/// A 32-bit IPv4 address.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ipv4Addr([u8; 4]);

impl Ipv4Addr {
    /// How many bytes the address occupies on the wire.
    pub const LEN: usize = 4;

    /// `0.0.0.0`, the address a host has before it is configured.
    pub const UNSPECIFIED: Ipv4Addr = Ipv4Addr([0, 0, 0, 0]);

    /// `255.255.255.255`, the limited broadcast address DHCP needs.
    pub const BROADCAST: Ipv4Addr = Ipv4Addr([255, 255, 255, 255]);

    /// `127.0.0.1`.
    pub const LOCALHOST: Ipv4Addr = Ipv4Addr([127, 0, 0, 1]);

    /// The address of these four bytes, in wire order.
    #[must_use]
    pub const fn new(first: u8, second: u8, third: u8, fourth: u8) -> Ipv4Addr {
        Ipv4Addr([first, second, third, fourth])
    }

    /// The address of these four bytes, in wire order.
    #[must_use]
    pub const fn from_octets(octets: [u8; 4]) -> Ipv4Addr {
        Ipv4Addr(octets)
    }

    /// The four bytes, in wire order.
    #[must_use]
    pub const fn octets(self) -> [u8; 4] {
        self.0
    }

    /// The address as one number, the first octet most significant. This
    /// is the form the prefix arithmetic of [`Ipv4Cidr`] works in.
    #[must_use]
    pub const fn to_bits(self) -> u32 {
        u32::from_be_bytes(self.0)
    }

    /// The address of this number, the most significant byte first.
    #[must_use]
    pub const fn from_bits(bits: u32) -> Ipv4Addr {
        Ipv4Addr(bits.to_be_bytes())
    }

    /// Whether this is `0.0.0.0`.
    #[must_use]
    pub const fn is_unspecified(self) -> bool {
        self.to_bits() == 0
    }

    /// Whether this is `255.255.255.255`. A directed broadcast, which is
    /// the last address of a network, is not this: it depends on a prefix,
    /// and [`Ipv4Cidr::broadcast`] is where that lives.
    #[must_use]
    pub const fn is_broadcast(self) -> bool {
        self.to_bits() == u32::MAX
    }

    /// Whether the address is in `127.0.0.0/8`.
    #[must_use]
    pub const fn is_loopback(self) -> bool {
        let [first, ..] = self.0;
        first == 127
    }

    /// Whether the address is in `224.0.0.0/4`.
    #[must_use]
    pub const fn is_multicast(self) -> bool {
        let [first, ..] = self.0;
        first & 0xF0 == 0xE0
    }

    /// The address written as four decimal octets joined by dots.
    ///
    /// # Errors
    ///
    /// [`WireError::Address`] when the text is not four groups, when a
    /// group is empty, longer than three digits, or not decimal, when a
    /// group exceeds 255, or when a group of more than one digit begins
    /// with a zero.
    pub fn parse(text: &str) -> Result<Ipv4Addr, WireError> {
        let mut octets = [0u8; 4];
        let mut groups = text.split('.');
        for slot in &mut octets {
            *slot = decimal_octet(groups.next().ok_or(WireError::Address)?)?;
        }
        if groups.next().is_some() {
            return Err(WireError::Address);
        }
        Ok(Ipv4Addr(octets))
    }
}

impl fmt::Display for Ipv4Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [first, second, third, fourth] = self.0;
        write!(f, "{first}.{second}.{third}.{fourth}")
    }
}

/// One decimal group of an IPv4 address, without a leading zero.
fn decimal_octet(group: &str) -> Result<u8, WireError> {
    let mut digits = group.bytes();
    let first = digits.next().ok_or(WireError::Address)?;
    let mut value = u16::from(digit(first)?);
    for byte in digits {
        if value == 0 {
            // A leading zero would let `010` and `10` be one address, and
            // would let a reader that knows C read the first as octal.
            return Err(WireError::Address);
        }
        let digit = u16::from(digit(byte)?);
        value = value
            .checked_mul(10)
            .and_then(|shifted| shifted.checked_add(digit))
            .ok_or(WireError::Address)?;
    }
    u8::try_from(value).map_err(|_| WireError::Address)
}

/// The number after the slash of a network text: decimal, without a
/// leading zero, and small enough to be a prefix length of some family.
/// Whether it is one of *this* family is the constructor's question.
fn decimal_prefix(text: &str) -> Result<u8, WireError> {
    let mut digits = text.bytes();
    let first = digits.next().ok_or(WireError::Address)?;
    let mut value = u16::from(digit(first)?);
    for byte in digits {
        if value == 0 {
            return Err(WireError::Address);
        }
        let digit = u16::from(digit(byte)?);
        value = value
            .checked_mul(10)
            .and_then(|shifted| shifted.checked_add(digit))
            .ok_or(WireError::Address)?;
    }
    u8::try_from(value).map_err(|_| WireError::Address)
}

/// One decimal digit as its value.
fn digit(byte: u8) -> Result<u8, WireError> {
    byte.checked_sub(b'0')
        .filter(|value| *value <= 9)
        .ok_or(WireError::Address)
}

/// An IPv4 address together with the length of its network prefix.
///
/// The address is kept as it was given: `192.168.1.7/24` is the interface
/// address seven on that network and not the network itself, and
/// [`network`](Ipv4Cidr::network) is what answers the second question.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ipv4Cidr {
    /// The address.
    address: Ipv4Addr,
    /// How many leading bits of it name the network, at most 32.
    prefix_len: u8,
}

impl Ipv4Cidr {
    /// How many bits an IPv4 prefix may cover.
    pub const MAX_PREFIX_LEN: u8 = 32;

    /// The address `address` on the network its first `prefix_len` bits
    /// name.
    ///
    /// # Errors
    ///
    /// [`WireError::PrefixLength`] when `prefix_len` is greater than 32.
    pub const fn new(address: Ipv4Addr, prefix_len: u8) -> Result<Ipv4Cidr, WireError> {
        if prefix_len > Ipv4Cidr::MAX_PREFIX_LEN {
            return Err(WireError::PrefixLength(prefix_len));
        }
        Ok(Ipv4Cidr {
            address,
            prefix_len,
        })
    }

    /// The address as it was given.
    #[must_use]
    pub const fn address(self) -> Ipv4Addr {
        self.address
    }

    /// How many leading bits name the network.
    #[must_use]
    pub const fn prefix_len(self) -> u8 {
        self.prefix_len
    }

    /// The mask with the prefix bits set.
    #[must_use]
    pub const fn netmask(self) -> Ipv4Addr {
        Ipv4Addr::from_bits(self.mask_bits())
    }

    /// The address with every host bit cleared.
    #[must_use]
    pub const fn network(self) -> Ipv4Addr {
        Ipv4Addr::from_bits(self.address.to_bits() & self.mask_bits())
    }

    /// The address with every host bit set, which is the directed
    /// broadcast address of the network.
    #[must_use]
    pub const fn broadcast(self) -> Ipv4Addr {
        Ipv4Addr::from_bits(self.address.to_bits() | !self.mask_bits())
    }

    /// Whether `address` is on this network. A prefix of zero contains
    /// every address, which is what makes the default route one of these.
    #[must_use]
    pub const fn contains(self, address: Ipv4Addr) -> bool {
        let mask = self.mask_bits();
        (address.to_bits() & mask) == (self.address.to_bits() & mask)
    }

    /// The network written as an address, a slash, and the prefix length.
    ///
    /// # Errors
    ///
    /// [`WireError::Address`] when there is no slash or the part before it
    /// is not an address, and [`WireError::PrefixLength`] when the part
    /// after it is a decimal number above 255 or not decimal at all, and
    /// [`WireError::PrefixLength`] when it is a number of at most 255 that
    /// is nonetheless longer than a prefix.
    pub fn parse(text: &str) -> Result<Ipv4Cidr, WireError> {
        let (address, prefix) = text.split_once('/').ok_or(WireError::Address)?;
        Ipv4Cidr::new(Ipv4Addr::parse(address)?, decimal_prefix(prefix)?)
    }

    /// The prefix as a mask. A prefix of 32 shifts by the whole width,
    /// which `checked_shr` reports rather than performs.
    #[expect(clippy::as_conversions, reason = "widening cast in a const fn")]
    const fn mask_bits(self) -> u32 {
        match u32::MAX.checked_shr(self.prefix_len as u32) {
            Some(host_bits) => !host_bits,
            None => u32::MAX,
        }
    }
}

impl fmt::Display for Ipv4Cidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.address, self.prefix_len)
    }
}

/// A 128-bit IPv6 address.
///
/// The text form is the canonical one of RFC 5952, section 4: leading
/// zeros suppressed, the longest run of zero groups replaced by `::` with
/// the leftmost run winning a tie, `::` never standing for a single zero
/// group, and lower case throughout. RFC 4291 permits several texts per
/// address and this type reads only the canonical one, so that an address
/// here has one spelling as every other address does (D-69). Upper case is
/// the one thing read and never written, as it is for [`MacAddr`].
///
/// The dotted form RFC 4291 allows for the last four bytes is not read. It
/// belongs to the IPv4-mapped addresses, which this system does not carry
/// as values at all, and accepting it would give `::ffff:1.2.3.4` and
/// `::ffff:102:304` to one address.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ipv6Addr([u8; 16]);

impl Ipv6Addr {
    /// How many bytes the address occupies on the wire.
    pub const LEN: usize = 16;

    /// How many 16-bit groups the text form is written in.
    pub const GROUPS: usize = 8;

    /// `::`, the address a host sends from before it has one.
    pub const UNSPECIFIED: Ipv6Addr = Ipv6Addr([0; 16]);

    /// `::1`.
    pub const LOCALHOST: Ipv6Addr = Ipv6Addr([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);

    /// `ff02::1`, every node on the link. A router advertisement arrives
    /// here, and this is where IPv6 sends what IPv4 would broadcast.
    pub const ALL_NODES: Ipv6Addr =
        Ipv6Addr([0xFF, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);

    /// `ff02::2`, every router on the link. A router solicitation goes
    /// here.
    pub const ALL_ROUTERS: Ipv6Addr =
        Ipv6Addr([0xFF, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]);

    /// The address of these sixteen bytes, in wire order. This is the
    /// constructor a `const` uses, because the one that takes groups
    /// cannot be `const`.
    #[must_use]
    pub const fn from_octets(octets: [u8; 16]) -> Ipv6Addr {
        Ipv6Addr(octets)
    }

    /// The address of these eight groups, each on the wire most
    /// significant byte first.
    #[must_use]
    pub fn new(groups: [u16; 8]) -> Ipv6Addr {
        let mut octets = [0u8; 16];
        let (pairs, _) = octets.as_chunks_mut::<2>();
        for (pair, group) in pairs.iter_mut().zip(groups) {
            *pair = group.to_be_bytes();
        }
        Ipv6Addr(octets)
    }

    /// The sixteen bytes, in wire order.
    #[must_use]
    pub const fn octets(self) -> [u8; 16] {
        self.0
    }

    /// The eight groups the text form is written in.
    #[must_use]
    pub fn groups(self) -> [u16; 8] {
        let mut groups = [0u16; 8];
        let (pairs, _) = self.0.as_chunks::<2>();
        for (group, pair) in groups.iter_mut().zip(pairs) {
            *group = u16::from_be_bytes(*pair);
        }
        groups
    }

    /// The address as one number, the first byte most significant. This is
    /// the form the prefix arithmetic of [`Ipv6Cidr`] works in.
    #[must_use]
    pub const fn to_bits(self) -> u128 {
        u128::from_be_bytes(self.0)
    }

    /// The address of this number, the most significant byte first.
    #[must_use]
    pub const fn from_bits(bits: u128) -> Ipv6Addr {
        Ipv6Addr(bits.to_be_bytes())
    }

    /// Whether this is `::`.
    #[must_use]
    pub const fn is_unspecified(self) -> bool {
        self.to_bits() == 0
    }

    /// Whether this is `::1`.
    #[must_use]
    pub const fn is_loopback(self) -> bool {
        self.to_bits() == 1
    }

    /// Whether the address is in `ff00::/8`. IPv6 has no broadcast
    /// address; what IPv4 broadcasts to, IPv6 sends to
    /// [`ALL_NODES`](Ipv6Addr::ALL_NODES), which is one of these.
    #[must_use]
    pub const fn is_multicast(self) -> bool {
        let [first, ..] = self.0;
        first == 0xFF
    }

    /// Whether the address is in `fe80::/10`, which is the range a host
    /// configures itself with before it has heard a router.
    #[must_use]
    pub const fn is_link_local(self) -> bool {
        let [first, second, ..] = self.0;
        first == 0xFE && second & 0xC0 == 0x80
    }

    /// Whether the address is in `fc00::/7`.
    #[must_use]
    pub const fn is_unique_local(self) -> bool {
        let [first, ..] = self.0;
        first & 0xFE == 0xFC
    }

    /// Whether the address names one interface, being neither multicast
    /// nor unspecified.
    #[must_use]
    pub const fn is_unicast(self) -> bool {
        !self.is_multicast() && !self.is_unspecified()
    }

    /// The solicited-node multicast address of this address: its low
    /// 24 bits appended to `ff02::1:ff00:0/104`, as RFC 4291,
    /// section 2.7.1 defines it.
    ///
    /// Neighbor Discovery asks for a neighbor in this group rather than at
    /// every node, which is why IPv6 needs no broadcast and why this
    /// belongs beside the address rather than in the crate that discovers.
    #[must_use]
    pub const fn solicited_node(self) -> Ipv6Addr {
        let [.., high, middle, low] = self.0;
        Ipv6Addr([
            0xFF, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0xFF, high, middle, low,
        ])
    }

    /// The address written in the canonical form of RFC 5952.
    ///
    /// # Errors
    ///
    /// [`WireError::Address`] when the text is not that form: a group of
    /// more than four hex digits or with a leading zero, more than one
    /// `::`, the wrong number of groups, a `::` standing for fewer than
    /// two zero groups or for a run that is not the longest and leftmost,
    /// or a run of two zero groups written out where a `::` belongs.
    pub fn parse(text: &str) -> Result<Ipv6Addr, WireError> {
        let (head, tail) = match text.split_once("::") {
            Some((head, tail)) => (head, Some(tail)),
            None => (text, None),
        };
        let mut groups = [0u16; 8];
        let head_len = group_count(head);
        let Some(tail) = tail else {
            if head_len != Ipv6Addr::GROUPS {
                return Err(WireError::Address);
            }
            for (slot, group) in groups.iter_mut().zip(head.split(':')) {
                *slot = hex_group(group)?;
            }
            // A run of two or more zero groups has a shorter spelling, and
            // RFC 5952, section 4.2.1 says that is the one to use.
            if longest_zero_run(groups).is_some() {
                return Err(WireError::Address);
            }
            return Ok(Ipv6Addr::new(groups));
        };
        if tail.contains("::") {
            return Err(WireError::Address);
        }
        let tail_len = group_count(tail);
        let written = head_len.checked_add(tail_len).ok_or(WireError::Address)?;
        // The `::` stands for at least two groups (RFC 5952, section
        // 4.2.2), so at most six may be written out.
        let compressed = Ipv6Addr::GROUPS
            .checked_sub(written)
            .filter(|hidden| *hidden >= 2)
            .ok_or(WireError::Address)?;
        if head_len > 0 {
            for (slot, group) in groups.iter_mut().zip(head.split(':')) {
                *slot = hex_group(group)?;
            }
        }
        if tail_len > 0 {
            for (slot, group) in groups.iter_mut().rev().zip(tail.rsplit(':')) {
                *slot = hex_group(group)?;
            }
        }
        // The `::` must cover the longest run, and the leftmost of two
        // equally long ones (RFC 5952, sections 4.2.1 and 4.2.3).
        if longest_zero_run(groups) != Some((head_len, compressed)) {
            return Err(WireError::Address);
        }
        Ok(Ipv6Addr::new(groups))
    }
}

impl fmt::Display for Ipv6Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let groups = self.groups();
        let Some((start, len)) = longest_zero_run(groups) else {
            return write_groups(f, &groups);
        };
        let end = start.saturating_add(len);
        write_groups(f, groups.get(..start).unwrap_or(&[]))?;
        f.write_str("::")?;
        write_groups(f, groups.get(end..).unwrap_or(&[]))
    }
}

/// Groups joined by colons, each in the shortest lower-case hex that
/// stands for it.
fn write_groups(f: &mut fmt::Formatter<'_>, groups: &[u16]) -> fmt::Result {
    let mut first = true;
    for group in groups {
        if !first {
            f.write_str(":")?;
        }
        first = false;
        write!(f, "{group:x}")?;
    }
    Ok(())
}

/// The longest run of two or more zero groups, as its first index and its
/// length, or `None` when there is none. A tie goes to the leftmost run,
/// which RFC 5952, section 4.2.3 requires.
fn longest_zero_run(groups: [u16; 8]) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    let mut start = 0usize;
    let mut len = 0usize;
    for (index, group) in groups.into_iter().enumerate() {
        if group == 0 {
            if len == 0 {
                start = index;
            }
            len = len.saturating_add(1);
            if len >= 2 && best.is_none_or(|(_, best_len)| len > best_len) {
                best = Some((start, len));
            }
        } else {
            len = 0;
        }
    }
    best
}

/// How many colon-separated groups a piece of an address text has.
fn group_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.split(':').count()
    }
}

/// One group of one to four hex digits, without a leading zero.
fn hex_group(text: &str) -> Result<u16, WireError> {
    let mut digits = text.bytes();
    let first = digits.next().ok_or(WireError::Address)?;
    let mut value = u16::from(hex_digit(first)?);
    for byte in digits {
        if value == 0 {
            // A leading zero would let `01` and `1` be one group, which
            // RFC 5952, section 4.1 forbids.
            return Err(WireError::Address);
        }
        let digit = u16::from(hex_digit(byte)?);
        value = value
            .checked_mul(16)
            .and_then(|shifted| shifted.checked_add(digit))
            .ok_or(WireError::Address)?;
    }
    Ok(value)
}

/// An IPv6 address together with the length of its network prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ipv6Cidr {
    /// The address.
    address: Ipv6Addr,
    /// How many leading bits of it name the network, at most 128.
    prefix_len: u8,
}

impl Ipv6Cidr {
    /// How many bits an IPv6 prefix may cover.
    pub const MAX_PREFIX_LEN: u8 = 128;

    /// The address `address` on the network its first `prefix_len` bits
    /// name.
    ///
    /// # Errors
    ///
    /// [`WireError::PrefixLength`] when `prefix_len` is greater than 128.
    pub const fn new(address: Ipv6Addr, prefix_len: u8) -> Result<Ipv6Cidr, WireError> {
        if prefix_len > Ipv6Cidr::MAX_PREFIX_LEN {
            return Err(WireError::PrefixLength(prefix_len));
        }
        Ok(Ipv6Cidr {
            address,
            prefix_len,
        })
    }

    /// The address as it was given.
    #[must_use]
    pub const fn address(self) -> Ipv6Addr {
        self.address
    }

    /// How many leading bits name the network.
    #[must_use]
    pub const fn prefix_len(self) -> u8 {
        self.prefix_len
    }

    /// The address with every host bit cleared.
    #[must_use]
    pub const fn network(self) -> Ipv6Addr {
        Ipv6Addr::from_bits(self.address.to_bits() & self.mask_bits())
    }

    /// Whether `address` is on this network. There is no broadcast address
    /// to go with it: IPv6 has none, so `Ipv6Cidr` has no `broadcast`
    /// where [`Ipv4Cidr`] has one.
    #[must_use]
    pub const fn contains(self, address: Ipv6Addr) -> bool {
        let mask = self.mask_bits();
        (address.to_bits() & mask) == (self.address.to_bits() & mask)
    }

    /// The network written as an address, a slash, and the prefix length.
    ///
    /// # Errors
    ///
    /// [`WireError::Address`] when there is no slash, when the part before
    /// it is not an address, or when the part after it is not a decimal
    /// number of at most 255; [`WireError::PrefixLength`] when that number
    /// is longer than a prefix.
    pub fn parse(text: &str) -> Result<Ipv6Cidr, WireError> {
        let (address, prefix) = text.split_once('/').ok_or(WireError::Address)?;
        Ipv6Cidr::new(Ipv6Addr::parse(address)?, decimal_prefix(prefix)?)
    }

    /// The prefix as a mask. A prefix of 128 shifts by the whole width,
    /// which `checked_shr` reports rather than performs.
    #[expect(clippy::as_conversions, reason = "widening cast in a const fn")]
    const fn mask_bits(self) -> u128 {
        match u128::MAX.checked_shr(self.prefix_len as u32) {
            Some(host_bits) => !host_bits,
            None => u128::MAX,
        }
    }
}

impl fmt::Display for Ipv6Cidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.address, self.prefix_len)
    }
}

/// Which of the two internet protocols an address belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IpVersion {
    /// IPv4.
    V4,
    /// IPv6.
    V6,
}

impl fmt::Display for IpVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IpVersion::V4 => f.write_str("IPv4"),
            IpVersion::V6 => f.write_str("IPv6"),
        }
    }
}

/// An address of either family.
///
/// This is the type the layers above the wire carry, so that a socket, a
/// route, and a neighbor are written once and not twice (D-69). There is
/// no IPv4-mapped form: the two families are distinct values throughout,
/// and an operation over one address of each is an error rather than a
/// conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IpAddr {
    /// An IPv4 address.
    V4(Ipv4Addr),
    /// An IPv6 address.
    V6(Ipv6Addr),
}

impl IpAddr {
    /// Which family the address belongs to.
    #[must_use]
    pub const fn version(self) -> IpVersion {
        match self {
            IpAddr::V4(_) => IpVersion::V4,
            IpAddr::V6(_) => IpVersion::V6,
        }
    }

    /// Whether this is an IPv4 address.
    #[must_use]
    pub const fn is_v4(self) -> bool {
        matches!(self, IpAddr::V4(_))
    }

    /// Whether this is an IPv6 address.
    #[must_use]
    pub const fn is_v6(self) -> bool {
        matches!(self, IpAddr::V6(_))
    }

    /// Whether the address is the unspecified one of its family.
    #[must_use]
    pub const fn is_unspecified(self) -> bool {
        match self {
            IpAddr::V4(address) => address.is_unspecified(),
            IpAddr::V6(address) => address.is_unspecified(),
        }
    }

    /// Whether the address is a loopback address of its family.
    #[must_use]
    pub const fn is_loopback(self) -> bool {
        match self {
            IpAddr::V4(address) => address.is_loopback(),
            IpAddr::V6(address) => address.is_loopback(),
        }
    }

    /// Whether the address is a multicast address of its family.
    #[must_use]
    pub const fn is_multicast(self) -> bool {
        match self {
            IpAddr::V4(address) => address.is_multicast(),
            IpAddr::V6(address) => address.is_multicast(),
        }
    }

    /// The address written in the canonical form of its family. A text
    /// with a colon in it is read as IPv6 and every other as IPv4, which
    /// is what makes the two forms one grammar.
    ///
    /// # Errors
    ///
    /// [`WireError::Address`] when the text is not the canonical form of
    /// the family it names itself.
    pub fn parse(text: &str) -> Result<IpAddr, WireError> {
        if text.contains(':') {
            Ipv6Addr::parse(text).map(IpAddr::V6)
        } else {
            Ipv4Addr::parse(text).map(IpAddr::V4)
        }
    }
}

impl From<Ipv4Addr> for IpAddr {
    fn from(address: Ipv4Addr) -> IpAddr {
        IpAddr::V4(address)
    }
}

impl From<Ipv6Addr> for IpAddr {
    fn from(address: Ipv6Addr) -> IpAddr {
        IpAddr::V6(address)
    }
}

impl fmt::Display for IpAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IpAddr::V4(address) => address.fmt(f),
            IpAddr::V6(address) => address.fmt(f),
        }
    }
}

/// A network of either family, which is what a routing table holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IpCidr {
    /// An IPv4 network.
    V4(Ipv4Cidr),
    /// An IPv6 network.
    V6(Ipv6Cidr),
}

impl IpCidr {
    /// Which family the network belongs to.
    #[must_use]
    pub const fn version(self) -> IpVersion {
        match self {
            IpCidr::V4(_) => IpVersion::V4,
            IpCidr::V6(_) => IpVersion::V6,
        }
    }

    /// The address as it was given.
    #[must_use]
    pub const fn address(self) -> IpAddr {
        match self {
            IpCidr::V4(network) => IpAddr::V4(network.address()),
            IpCidr::V6(network) => IpAddr::V6(network.address()),
        }
    }

    /// How many leading bits name the network.
    #[must_use]
    pub const fn prefix_len(self) -> u8 {
        match self {
            IpCidr::V4(network) => network.prefix_len(),
            IpCidr::V6(network) => network.prefix_len(),
        }
    }

    /// Whether `address` is on this network. An address of the other
    /// family is not, which is what keeps a v4 route from matching a v6
    /// destination in a table that holds both.
    #[must_use]
    pub const fn contains(self, address: IpAddr) -> bool {
        match (self, address) {
            (IpCidr::V4(network), IpAddr::V4(address)) => network.contains(address),
            (IpCidr::V6(network), IpAddr::V6(address)) => network.contains(address),
            _ => false,
        }
    }

    /// The network written as an address, a slash, and the prefix length.
    ///
    /// # Errors
    ///
    /// What [`Ipv4Cidr::parse`] and [`Ipv6Cidr::parse`] return.
    pub fn parse(text: &str) -> Result<IpCidr, WireError> {
        if text.contains(':') {
            Ipv6Cidr::parse(text).map(IpCidr::V6)
        } else {
            Ipv4Cidr::parse(text).map(IpCidr::V4)
        }
    }
}

impl From<Ipv4Cidr> for IpCidr {
    fn from(network: Ipv4Cidr) -> IpCidr {
        IpCidr::V4(network)
    }
}

impl From<Ipv6Cidr> for IpCidr {
    fn from(network: Ipv6Cidr) -> IpCidr {
        IpCidr::V6(network)
    }
}

impl fmt::Display for IpCidr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IpCidr::V4(network) => network.fmt(f),
            IpCidr::V6(network) => network.fmt(f),
        }
    }
}

/// A transport port, of UDP or of TCP.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Port(u16);

impl Port {
    /// Port zero, which names no service. UDP writes it in the source
    /// field of a datagram that expects no reply.
    pub const UNSPECIFIED: Port = Port(0);

    /// The first port of the dynamic range of RFC 6335, which is where an
    /// ephemeral port is drawn from.
    pub const EPHEMERAL_FIRST: Port = Port(49152);

    /// The last port of that range.
    pub const EPHEMERAL_LAST: Port = Port(65535);

    /// The port of this number.
    #[must_use]
    pub const fn new(number: u16) -> Port {
        Port(number)
    }

    /// The number.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    /// Whether this is port zero.
    #[must_use]
    pub const fn is_unspecified(self) -> bool {
        self.0 == 0
    }

    /// Whether the port is in the dynamic range a client draws from. The
    /// range ends at the last port there is, so only its start is a test.
    #[must_use]
    pub const fn is_ephemeral(self) -> bool {
        self.0 >= Port::EPHEMERAL_FIRST.0
    }
}

impl fmt::Display for Port {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
