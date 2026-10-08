# Original protocol and progs checksums

`qa_core::checksum` contains one MD4 implementation and one CRC-CCITT
implementation. `block_checksum` reduces the MD4 state with the original four
word XOR. `block_checksum_key` prefixes the little-endian four-byte key as
original Q3 does. `crc_block` starts at `0xffff`, uses polynomial `0x1021` and
has no final XOR; progs CRC and the protocol callers use this same function.

QW and Q2 sequence checksums share CRC and each retain their native sequence
salt table. Both cap payloads at 60 bytes; QW mixes the low sequence bytes into
the salt, while Q2 XORs the byte sum into its CRC. These functions use stack
state and bounded arrays. There is no full-message padding allocation.

These are original format/wire contracts. They are not content identifiers,
cache keys, change detectors or save fingerprints. No SHA implementation or
dependency is introduced. The rule checker rejects another MD4 declaration
outside the core checksum module, and the verification tool plants a duplicate
to prove that guard rejects it before compilation.

## Verification

```sh
cargo test -p qa-core --test checksum
python3 tools/check_checksums.py --qsrc "$QA_QSRC" --output "$QA_EVIDENCE"
python3 tools/verify_rules.py --evidence "$QA_RULE_EVIDENCE"
```

The comparison compiles original Q3 MD4, Q2 CRC and original QW/Q2 sequence
routines, with MD4's `UINT4` explicitly fixed to its original 32-bit ABI width
on this 64-bit host. Algorithms are unchanged. The first 130 input lengths
cover zero, padding and block boundaries; another 10,000 seeded inputs vary
length, key and sequence. All 50,650 checksum results match bit for bit.
Fixtures also reduce RFC 1320 vectors, check CCITT `123456789 = 0x29b1`, keyed
prefix equivalence and the native 60-byte sequence limit.

These checks prove checksum arithmetic and the structural guard, not pure
server admission, multiplayer negotiation or QC module loading. Those callers
are wired in later milestones.

## Sources and attribution

This implementation is derived from the RSA Data Security, Inc. MD4
Message-Digest Algorithm. Its copyright, redistribution and warranty notices
are retained in `crates/core/src/checksum.rs` and extracted C evidence.
Original references: `quake-iii-arena/code/qcommon/md4.c`,
`quake-2/qcommon/crc.c`, `quake-2/qcommon/common.c` and
`quake/QW/client/common.c`. Proven Q2 sequence behavior was checked in the C
port's `src/network/q2/checksum.c`.

Authorised retired algorithms inspected only at
`muse-final:crates/content/src/hash.rs` (MD4 section) and
`muse-final:crates/guest/src/qc/program.rs` (CRC16 section). Retired hashing,
file identity and allocating whole-message padding were excluded.
