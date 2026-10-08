# Product detection and startup assets

`qa_content::products` discovers stock products from mounted directory metadata
and each product's startup-map witness. Recipes have static numeric ProductIds,
edition metadata, a directory name, an optional base dependency and a default
map. They contain no game-family compatibility filter. Movement, weapons,
modules and rendering remain independent choices using common services.

Map references stay scoped to the product directory, even when the common VFS
contains several `maps/start.bsp` files. Within that directory normal mount
priority wins. Standalone `id1`, `baseq2` and `baseq3` directories also work;
the two `rogue` packs are disambiguated by their path ancestry and their own
map witness. Rerelease directory ancestry is explicit metadata. No content
digest, edition fingerprint or whole-asset snapshot is involved.

Classic/rerelease Q1 campaign defaults use `start`; classic CTF uses
`ctfstart`, and rerelease CTF uses its owned `ctf1`. Q2 defaults use `base1`,
`xswamp`, `rmine1`, `q2ctf1`, `lmctf09`, `mguhub` or `q64/rtest` according to
the product. Q3 uses `q3dm1`, Team Arena uses `mpteam1`, and Quake Live uses
`campgrounds`. These are startup choices, not restrictions on maps or mixes.
QuakeWorld shares original Quake assets and is exposed as an explicit shared
asset recipe; it does not require a second copy of the Quake PAKs.

Directory and archive mount ids describe inventory membership. Launch setup
must mount the selected directories and base dependencies with the common
VFS's `mount_product`; a broad inventory directory mount is not a scoped
launch configuration. Custom/mod content remains accessible through the same
VFS independently of stock-product detection.

## Verification

```sh
cargo test -p qa-content --test products
cargo run --release -p qa-content --example products -- "$QA_QFILES"
```

The headless probe resolves each default, then mounts that product directory
alone and fully parses the default BSP geometry. Its output labels the scope
as resolution rather than a game launch. The owned inventory resolves 26 recipes:
25 asset installations and QuakeWorld's shared-asset recipe. All 26 isolated
default maps parse successfully. THE-784 remains In Progress until
the app loads and starts each selected product's default map in a private run.
No renderer, movement or qualified installation is proved here.

Sources: C `src/content/catalog/products.c` supplies directory, pack and base
relationships; `src/session/configuration/presets.c` supplies campaign map
choices. Their family/provider structure was excluded. Q3 `q3dm1` follows
THE-784's requested default, and rerelease CTF `ctf1` is an explicit startup
choice from its owned maps. No retired catalog implementation was mined.
