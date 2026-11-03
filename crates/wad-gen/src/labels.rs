//! The labels images and containers carry, v1's keys exactly: wadd, CI and
//! `wadspaces-pkg` read them, so none of them changes.

/// A package's id (its Containerfile sets it; CI's and a machine's builds too).
pub const PKG: &str = "io.wadspaces.pkg";
/// The packages a package was built on, `a,b` (its Containerfile sets it).
pub const PKG_NEEDS: &str = "io.wadspaces.pkg.needs";
/// The base's ABI a package was built for (CI's push sets it, and a build here).
pub const PKG_ABI: &str = "io.wadspaces.pkg.abi";
/// The version it installed (CI's push, from its meta.json).
pub const PKG_VERSION: &str = "io.wadspaces.pkg.version";
/// A hash of its files (CI's push: unchanged content isn't pushed again).
pub const PKG_CONTENT: &str = "io.wadspaces.pkg.content";
/// Built on this machine, not by CI ("true").
pub const PKG_BUILT_HERE: &str = "io.wadspaces.pkg.built-here";
/// The base's ABI: packages built on a base with the same one fit it.
pub const ABI: &str = "io.wadspaces.abi";
/// A base that takes a wadspace's extras when it starts.
pub const EXTRAS: &str = "io.wadspaces.extras";
/// A chain image's packages so far (`id@image-id-12,…`, bottom first).
pub const CHAIN: &str = "io.wadspaces.chain";
/// A set's packages, as it stacks them (the same form).
pub const PKGS: &str = "io.wadspaces.pkgs";
/// A set: packages put together ("1").
pub const ASSEMBLY: &str = "io.wadspaces.assembly";
/// The id of the image it was built FROM (a chain's or a set's).
pub const FROM: &str = "io.wadspaces.from";
/// What an image is: `set:<key>`.
pub const LAYER: &str = "io.wadspaces.layer";
/// A set's key again (what wadd lists images by).
pub const SHA: &str = "io.wadspaces.dockerfile-sha256";

/// A wadspace's containers: its id.
pub const WADSPACE: &str = "wadspaces.id";
/// A stream sidecar's role ("display").
pub const ROLE: &str = "wadspaces.role";
/// A stream sidecar's stream (its guid).
pub const STREAM: &str = "wadspaces.stream";
