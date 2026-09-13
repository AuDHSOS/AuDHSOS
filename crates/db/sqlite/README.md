# db-sqlite

The SQLite file format as logic: the hundred-byte header, the b-tree pages
over it, the cells on those pages, the overflow chains they continue on,
and the record format one row is.

This is the first layer of a SQLite in Rust. It reads; it allocates
nothing; it is `no_std` and `forbid(unsafe_code)`; and it borrows every
value out of the bytes it was given, so a database is read where it lies.

```rust
use db_sqlite::{Error, Image, Value};

/// Every table of a database, by the name the schema gives it.
fn table_names(bytes: &[u8]) -> Result<(), Error> {
    let image = Image::open(bytes)?;
    for row in image.schema() {
        // type, name, tbl_name, rootpage, sql
        let record = row?.record()?;
        if record.value(0)? == Some(Value::Text(b"table"))
            && let Some(Value::Text(name)) = record.value(1)?
        {
            // `name` is in the encoding the header names, which is why it
            // is bytes and not a string.
            let _ = name;
        }
    }
    Ok(())
}
```

## What it holds to

Every structure here is written against the format's own document, which
is kept beside the code in [`docs/sqlite/`](../../../docs/sqlite/README.md)
because a constant without its sentence is a constant nobody can check:

- The header, section 1.3. The three fixed fractions (64, 32, 32) are
  checked rather than assumed, and a page size that is not a power of two
  between 512 and 65536 is refused. The value 1 in the page size field
  means 65536.
- B-tree pages, section 1.6: the four page types, the cell pointer array,
  and the four cell shapes. Page 1 carries the database header before its
  b-tree header, which is the one place the layout differs per page.
- Overflow, section 1.6 again: `X`, `M` and the remainder rule that decides
  how much of a payload stays on its page, so that the last overflow page
  is as full as it can be.
- The record format, section 2.1: the header of serial types, the body
  they describe, and the two types (10 and 11) the format reserves.

## What it refuses

A refusal names the rule of the format that was broken, not a symptom.
A cell pointer outside the usable part of a page, a varint that does not
end, a serial type the format reserves, a chain of overflow pages longer
than the file has pages: each is its own variant of `Error`.

Text is answered as the bytes it is stored in. The encoding is in the
header, and this layer does not convert, because a converting reader would
have to allocate and this one does not.

## What it costs

Opening a database reads a hundred bytes. A page is found by multiplying,
so a page costs O(1). Walking a table of `n` rows costs O(n) steps and one
frame per level of the tree, at most 32 levels and no allocation. Reading a
payload that overflows costs one page read per link of its chain.

## What is not here yet

Writing, the free list, pointer maps, the write-ahead log, and the index
b-trees beyond reading their pages. The cursor walks a table tree in rowid
order; an index walk, the pager, and everything above it come next.
