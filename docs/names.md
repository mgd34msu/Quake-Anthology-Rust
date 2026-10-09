# Shared name storage

Core `NameTable` stores byte-exact names once and assigns `NameId(u32)` values.
Empty is ID zero; absent fields remain `None`. Raw bytes, embedded NULs and case
variants retain separate exact identities. Native C-string/module/wire adapters
still apply their original string representation and limits.

Initial IDs follow exact byte order. Registration into storage reserved at load
appends stable IDs without moving any prior name or changing its folded group.
The exact index remains sorted; one bucket index supplies ASCII-folded lookup.
Bucket keys only accelerate equality lookup. They are never content identities.
Capacity failure leaves earlier names and IDs intact and rejects that registration.

Console cvars, commands and aliases share that implementation and one registry
arena. Commands resolve a token once before numeric dispatch. Command listing
uses the core Q3 `Q_stricmp` comparison order; flag members and native default
roles resolve at load. Alias text and registration storage remain fixed during
play. A removed alias releases its slot; interned names retain their session
lifetime. Current console load reserves 32,768 registrations and 1 MiB of bytes.

Renderer assets share one name arena for material and image keys. The single
`canonical_path` function converts backslashes to slashes and folds ASCII case,
leaving non-ASCII bytes unchanged. `intern_path` creates the canonical exact ID
at registration, with a cached canonical ID per folded group. Exact entity names
never inherit this path rule. Repeated path lookup and resource reuse allocate
nothing. Current asset load reserves 65,536 registrations and 4 MiB of bytes.

Material records keep numeric names and resolved stages/settings; the same name
with different resolved images or settings still yields distinct materials.
Image keys combine a NameId with typed recipe data for palette, transparency,
optional RGBA override and native image-use rules. Q3 first-registration sampler
ownership and conflict reporting are preserved. Shader grammar and source
diagnostics remain cold load data.

The original trigger/door and combined-entity installed acceptance for THE-617
remains open. Headless comparisons and allocation probes do not establish that
runtime feature; measurements and their scopes are recorded in frame-times.md.
