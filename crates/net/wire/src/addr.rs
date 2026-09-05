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
    pub fn is_unspecified(self) -> bool {
        self == Ipv4Addr::UNSPECIFIED
    }

    /// Whether this is `255.255.255.255`. A directed broadcast, which is
    /// the last address of a network, is not this: it depends on a prefix,
    /// and [`Ipv4Cidr::broadcast`] is where that lives.
    #[must_use]
    pub fn is_broadcast(self) -> bool {
        self == Ipv4Addr::BROADCAST
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
        let address = Ipv4Addr::parse(address)?;
        let mut digits = prefix.bytes();
        let first = digits.next().ok_or(WireError::Address)?;
        let mut prefix_len = u16::from(digit(first)?);
        for byte in digits {
            if prefix_len == 0 {
                return Err(WireError::Address);
            }
            let digit = u16::from(digit(byte)?);
            prefix_len = prefix_len
                .checked_mul(10)
                .and_then(|shifted| shifted.checked_add(digit))
                .ok_or(WireError::Address)?;
        }
        let prefix_len = u8::try_from(prefix_len).map_err(|_| WireError::Address)?;
        Ipv4Cidr::new(address, prefix_len)
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
