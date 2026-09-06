// SPDX-License-Identifier: AGPL-3.0-only
// Copyright (C) 2026 Manuel Baesler and contributors

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![doc = include_str!("../README.md")]

pub mod error;
pub mod message;
pub mod name;
pub mod record;
pub mod resolver;

pub use error::DnsError;
pub use message::{
    Flags, HEADER_LEN, Header, MAX_MESSAGE_LEN, MAX_QUERY_LEN, Message, Opcode, Question,
    Questions, Records, ResponseCode, write_query,
};
pub use name::{MAX_LABEL_LEN, MAX_NAME_LEN, Name};
pub use record::{Class, Record, RecordData, RecordType};
pub use resolver::{
    Config, MAX_ADDRESSES, MAX_CNAME_LINKS, MAX_SERVERS, Query, Resolver, SERVER_PORT, Status,
};

#[cfg(test)]
mod tests;
