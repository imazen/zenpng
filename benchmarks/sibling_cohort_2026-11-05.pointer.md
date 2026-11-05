# Shared sibling cohort qualification (2026-11-05)

Base: `a5eeaddb3c5efc92c9ff6b3e08e3124b463f0c14`.
Manifests now select zencodec family
`1816b4d082234c04c3805411b437b8396040f522` and zenpixels
`fa836abf72afe9826e866dc25712b7c800323962` in both root and fuzz graphs.
No native decoder or encoder algorithm changed. A private APNG test helper
visits reference frames instead of retaining all of them.

Raw logs and hashes, checked on dev:
`/mnt/v/output/zensim-siblings-20261105/zenpng/`.
Validation ran on a Ryzen 9 7950X under WSL2 in an independent source copy and Cargo output
directory, with TMPDIR under the home directory, jobs=2, serial Rust tests,
and the shared heavy lock. No primary sibling checkout was used.

Commands: `just sibling-cohort-check` is the aggregate gate. Its three disjoint
parts were run separately so the large static oracle completed only once:
`just sibling-png-reference-check`, `just sibling-apng-roundtrip-check`, then
`just sibling-cohort-rest-check`.
The former retains the original corrupt-input exclusions; the latter excludes
only those already-completed tests by their exact names, then runs strict all-target,
all-feature clippy. Existing ignored tests remain ignored.

Static oracle: 18,485 matches, 15 existing exclusions, two both-error inputs,
zero failures; the separate image-png fixture directory was absent. Receipt:
`rc=0 680s | peak-RSS 5.17GiB | min-avail 2677MiB | peak-load 5.98`.
The earlier 4 GiB-cap run failed because its cgroup exhausted memory; it is
preserved, and no assertion was removed to obtain the passing 6 GiB run.

Focused APNG control (`just sibling-apng-reference-check`): old cohort
(core `61fb96f1`, pixels `f23ac753`) and new cohort both report 26 matches,
72 mismatches, three native decode errors and zero reference errors. The
existing test asserts at least ten matches, not zero mismatches. It compares
reference raw subframes against native composited frames, so this is an
unchanged-outcome control and not proof of APNG compositing correctness.
New-cohort receipt: `rc=0 20s | peak-RSS 2.69GiB`.
Old-cohort receipt: `rc=0 53s | peak-RSS 2.69GiB`.

APNG roundtrip: its existing frame-count and pixel assertions pass after
releasing duplicate native input storage before encoding. An additional frame
length assertion prevents zip truncation. The test uses the captured RGBA8
input values as its exact expected pixels. Receipt:
`rc=0 120s | peak-RSS 6.27GiB | min-avail 2294MiB | peak-load 2.87`
under an 8 GiB hard cap. Preserved earlier runs exhausted a 6 GiB cap or hit
the validation filesystem's documented ENOMEM read failure. No input was
removed and no decoder, encoder or expectation was weakened.
