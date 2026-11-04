# Spike: upstream fixes found by the crosscheck

**Date:** 2026-11-04 · **Outcome:** 3 small upstream changes planned: `re_ros_msg`, `blf_asc`, `falcon_mdf`; none for canboat. Wise Bucket's workarounds stay until each is released.

## Question

`evals/crosscheck.sh` found data that three reader libraries drop, decode wrongly without a sign, or leave out. Wise Bucket now works around them (ADR 0034, ADR 0035). Which upstream changes would replace those workarounds with exact data, and would the projects take them?

## What was found

1. **The projects:**
   - **rerun** (`re_ros_msg`): very active. Small PRs (about 100 lines) are welcome without prior discussion. The contributor writes the PR description, must understand an agent-written change, and states their confidence; prototypes go in draft.
   - **`blf_asc`** (chirikoro, Apache-2.0): last push 2026-03-26, no issue or pull request ever.
   - **`falcon_mdf`** (Apache-2.0): active (last push 2026-10-27); the maintainer merges their own PRs.
   - **Releases:** `deny.toml` denies git dependencies (`unknown-git = "deny"`), so a fix is usable only once released on crates.io.
2. **`re_ros_msg` 0.38.1:**
   - `decode_bytes_into_arrow` (`crates/utils/re_ros_msg/src/reflection/cdr_to_arrow.rs`) drops its `re_cdr::CdrReader`, which has `remaining()`. A definition that doesn't match its payload therefore decodes without a sign: the db3 sample's cut-down `rcl_interfaces/msg/Log` (`uint8 level`) gives a `/rosout` level of 251 instead of 20, with 171 of 172 bytes unread.
   - rerun's `main` (2026-11-04) is unchanged there.
3. **`blf_asc` 0.2.0:**
   - An ASC remote frame written without a length (`2.510001 2 100 Tx r`, 5 tokens) hits `tokens.len() < 6 → continue`: python-can reads 20 frames in `logfile.asc`, `blf_asc` 19.
   - Lines it skips leave no trace, and only `AscReader::open` from a path exists. Wise Bucket therefore matches frames to lines by time, which is wrong when a dropped frame line shares its time with the next frame.
   - BLF objects of other types are skipped without a count.
   - A CAN FD line with 0 data bytes becomes a remote frame. python-can does the same, so this is not reported.
4. **`falcon_mdf` 0.7.1:**
   - The MDF4 sample's header comment is an MD block holding plain text, not XML. `falcon_mdf` returns no comment; asammdf returns the text.
   - The channel group's `TZNSSNK` is its comment: the group has no acquisition name, so `group_1` is right.
5. **canboat 8.3.0:** Wise Bucket counts the fast-packet frames that end up in no message exactly, with canboat's public `Reassembler`. A counter in canboat would add nothing.

## Decision

- **Order, by value:** `re_ros_msg`, then `blf_asc`, then `falcon_mdf`.
- **Non-breaking additions only** (new methods, no changed signatures), so each crate can release them as a minor version.
- **Preparation:** Claude prepares each change in a clone outside the repository: the change, a test, and a draft PR text. The maintainer reviews it, writes the final PR description, and gives a go before each fork, issue or PR.
- **No fork is published** if a project doesn't answer: Wise Bucket's workarounds stay.

### 1. `re_ros_msg`: the bytes left after decoding

- **Change:** keep the bytes left after the last decoded message, with a new `CdrArrowDecoder::decode_message_with_remaining(&mut self, buf) -> Result<usize, CdrDecodeError>` (or a `last_remaining()` accessor). `decode_message` stays as it is.
- **Test:** a two-field definition reading a payload with 8 more bytes reports 8; a matching payload reports at most its alignment padding.
- **Wise Bucket follow-up:**
  - Topics with messages left with more than 7 bytes get a note: "the definition doesn't match the payload: N of M bytes not decoded in K messages".
  - The `ros2_db3` sample test asserts that note.

### 2. `blf_asc`: what the reader skips

- **One PR:**
  - read remote frames written without a length, with dlc 0;
  - add `AscReader::line_number()`, the line of the message just returned;
  - add `BlfReader::skipped_objects()`, counts by object type.
- **Tests:** the remote line, line numbers over a small log, and a BLF file with an object of an unread type.
- **Wise Bucket follow-up:**
  - Match ASC lines by `line_number()`, which removes `ASC_LOOKAHEAD` and its `ponytail:` in `crates/wb-data/src/can.rs`.
  - Put the skipped BLF objects in the notes, by type.
  - The `can` sample test expects 20 frames and 19 `raw` rows.

### 3. `falcon_mdf`: a header comment written as plain text

- **Change:** an MD block that isn't XML is returned as text.
- **Test:** a file whose HD comment is a plain-text MD block.
- **Wise Bucket follow-up:** the `mdf4` sample test asserts `properties.comment`.

## Possible improvements

- **CAN FD remote rule:** if python-can changes how it reads a CAN FD line with 0 data bytes, `blf_asc` could follow.
- **One commit per release:** each Wise Bucket follow-up is a single commit after the release that bumps the dependency, with its sample-test assertions.
