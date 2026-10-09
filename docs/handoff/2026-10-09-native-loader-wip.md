# Native loader parked for the owner core-first order

Verified main is `230ac80c0b3d6c9a261df4aab759d39271502e97` (THE-2563/THE-796/THE-863). It contains the inert PE reader and original-C comparisons. It does not execute native modules.

This branch parks unfinished THE-2575 ELF work. It adds explicit library/program load roles, checked ELF32/ELF64 x86 load mappings, dynamic tags, needed names, TLS and RELRO metadata. Symbol lookup, ELF relocations, import binding, initializers and native execution are absent. The public native-image parse signature changes on this branch only.

Partial checks before parking: six focused native-image tests passed; the debug original-C mapping/dynamic comparator passed four library/program cases (196,864 compared bytes) for `qa-native-profile.so`. Evidence is under `~/.cache/qa-rust/THE-2575-elf-20261009/reference/`. Workspace tests, Clippy, release qualification and allocation checks have not been run on this ELF slice. Do not treat it as verified or merge it on that basis.

Resume native work only after the owner core-first order permits it. Recheck the branch against main, complete symbol/relocation semantics, and run the required checks before adoption. No native instructions, constructors or module game functions have been executed by this reader.
