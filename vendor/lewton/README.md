This is the packet decoder from Lewton 0.10.2, adapted to retain DSP storage
allocated from the identification and setup headers. The MIT license is used;
the original copyright and both offered licenses are retained in `LICENSE`.

There is one packet-decoding implementation. The allocating container reader,
C bindings and unused output adapters are omitted. Ogg framing belongs to
the engine's shared sound reader. The original registry release is used only
as an independent test reference.

`PacketDecoder` owns its headers and scratch. Reads cannot substitute a
different setup. Reset clears decoded state while retaining allocated storage.
Floor, residue, transform, overlap and integer conversion operations preserve
the original arithmetic order for the comparison corpus. The adaptation also
fixes first-order and wide-amplitude floor-zero cases, rejects a codebook index
equal to the book count, and preserves requested channel order when borrowing
a coupling pair. Mathematical regressions cover the floor-zero cases; PCM
comparisons cover the original generated fixtures and retail track.
