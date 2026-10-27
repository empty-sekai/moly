# Empty mesh copy guard

This is the crates.io `bevy_render` 0.19.1 source, licensed under MIT or
Apache-2.0. The original license files accompany it.

The crate archive SHA-256 is
`fc86a32b2eef9859e1e7d323c6d1a9150fe77aa17c30fd05abebc12f645929dc`.

The only Rust change backports Bevy commit
`3beb244c690865f78ab9fa2a1298c2540730d5f3`,
[PR #24960](https://github.com/bevyengine/bevy/pull/24960).
The mesh allocator already skips allocation for empty vertex buffers. Its copy
pass must apply the same guard; otherwise empty UI meshes access an unallocated
slab key when views are created or cleared. Nonempty mesh validation is retained.

Trailing whitespace in `color_operations.wgsl` and `maths.wgsl` is normalized
for the repository's whitespace check; shader behavior is unchanged.

Remove this override after upgrading to a Bevy release containing that fix.
