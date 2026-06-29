//! Environment classification axes — the typed vocabulary `kanchi` probes the
//! host for, and the inputs a shikumi `DiscoveryLayer` maps into best-fit
//! config defaults.
//!
//! Each axis is a small **closed enum** carrying a `const ALL` (every variant,
//! declaration order), a canonical `as_str` / `FromStr` / `Display` string
//! round-trip, and — where the fact is probe-able **without a cluster client**
//! — a host `detect() -> Option<Self>`. The cluster-API-requiring axes
//! ([`CapacityType`], [`Tenancy`]) carry only the vocabulary here; a
//! cluster-aware consumer (e.g. breathe, which holds a kube client + can run a
//! `SelfSubjectAccessReview`) fills them. Kept serde-free so `kanchi` stays the
//! lean detection leaf — string round-trip is via `as_str`/`FromStr`, which a
//! consumer uses to project an axis into a `figment` config dict.

/// Declare a closed classification axis: the enum + `const ALL` +
/// `as_str`/`Display`/`FromStr` over the canonical tags. Single source of the
/// wire string (the literal), so the tag never drifts from the round-trip.
macro_rules! closed_axis {
    (
        $(#[$m:meta])*
        $name:ident { $( $(#[$vm:meta])* $variant:ident = $tag:literal ),+ $(,)? }
    ) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name { $( $(#[$vm])* $variant ),+ }

        impl $name {
            /// Every variant, in declaration order.
            pub const ALL: &'static [Self] = &[ $( Self::$variant ),+ ];

            /// The canonical lowercase tag for this variant.
            #[must_use]
            pub const fn as_str(self) -> &'static str {
                match self { $( Self::$variant => $tag ),+ }
            }
        }

        impl ::core::fmt::Display for $name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl ::core::str::FromStr for $name {
            type Err = ();
            fn from_str(s: &str) -> ::core::result::Result<Self, ()> {
                match s { $( $tag => Ok(Self::$variant), )+ _ => Err(()) }
            }
        }
    };
}

closed_axis! {
    /// The host operating system. Detected from the compiled target (which, for
    /// a running binary, IS the host it runs on).
    Os { MacOs = "macos", Linux = "linux", Other = "other" }
}

impl Os {
    /// The host OS, from `std::env::consts::OS`.
    #[must_use]
    pub fn detect() -> Self {
        match std::env::consts::OS {
            "macos" => Self::MacOs,
            "linux" => Self::Linux,
            _ => Self::Other,
        }
    }
}

closed_axis! {
    /// The host CPU architecture.
    Arch { X86_64 = "x86_64", Aarch64 = "aarch64", Other = "other" }
}

impl Arch {
    /// The host arch, from `std::env::consts::ARCH`.
    #[must_use]
    pub fn detect() -> Self {
        match std::env::consts::ARCH {
            "x86_64" => Self::X86_64,
            "aarch64" => Self::Aarch64,
            _ => Self::Other,
        }
    }
}

closed_axis! {
    /// Whether the process runs under a Kubernetes orchestrator. `Bare` covers
    /// any non-cluster host (laptop, VM, bare metal, CI without a cluster).
    Orchestrator { Kubernetes = "kubernetes", Bare = "bare" }
}

impl Orchestrator {
    /// `Kubernetes` iff [`crate::probe::in_cluster`] reports an in-cluster
    /// context (service-account injection), else `Bare`.
    #[must_use]
    pub fn detect() -> Self {
        if crate::probe::in_cluster() == Some(true) {
            Self::Kubernetes
        } else {
            Self::Bare
        }
    }
}

closed_axis! {
    /// The cloud the host runs in. `None` = not a recognized cloud (homelab,
    /// bare metal, kind). Detection here is cheap env-hint only; an
    /// IMDS-backed probe is a documented later enhancement, and a cluster-aware
    /// consumer can refine this from a node's `providerID`.
    Cloud { Aws = "aws", Gcp = "gcp", Azure = "azure", None = "none" }
}

impl Cloud {
    /// Best-effort cloud from cheap environment hints (no network). `None` when
    /// no hint is present — callers that need certainty use IMDS or node
    /// `providerID` (cluster-aware consumer). Never blocks.
    #[must_use]
    pub fn detect() -> Self {
        let has = |k: &str| std::env::var_os(k).is_some();
        // AWS injects AWS_EXECUTION_ENV / AWS_REGION broadly; GCP sets a
        // metadata host hint; Azure exposes an MSI endpoint.
        if has("AWS_EXECUTION_ENV") || has("AWS_REGION") || has("AWS_DEFAULT_REGION") {
            Self::Aws
        } else if has("GCE_METADATA_HOST") || has("GOOGLE_CLOUD_PROJECT") {
            Self::Gcp
        } else if has("MSI_ENDPOINT") || has("IDENTITY_ENDPOINT") {
            Self::Azure
        } else {
            Self::None
        }
    }
}

closed_axis! {
    /// A node's market posture. **Vocabulary only** — not host-probeable; a
    /// cluster-aware consumer fills it from a node label
    /// (`karpenter.sh/capacity-type`, `eks.amazonaws.com/capacityType`).
    CapacityType { Spot = "spot", OnDemand = "on-demand", Unknown = "unknown" }
}

closed_axis! {
    /// Whether the workload runs in its OWN cluster or a FOREIGN (e.g.
    /// multi-tenant, someone-else-owned) one — the axis that drives a
    /// least-privilege default posture. **Vocabulary only** — a cluster-aware
    /// consumer derives it (the strongest `Foreign` signal is a denied
    /// `SelfSubjectAccessReview` for cluster-wide mutation). `Unknown` until
    /// probed.
    Tenancy { Own = "own", Foreign = "foreign", Unknown = "unknown" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::str::FromStr;

    #[test]
    fn closed_axes_round_trip_every_variant() {
        macro_rules! rt {
            ($t:ty) => {
                for v in <$t>::ALL {
                    assert_eq!(<$t>::from_str(v.as_str()), Ok(*v), "round-trip {v}");
                }
                assert!(<$t>::from_str("definitely-not-a-tag").is_err());
            };
        }
        rt!(Os);
        rt!(Arch);
        rt!(Orchestrator);
        rt!(Cloud);
        rt!(CapacityType);
        rt!(Tenancy);
    }

    #[test]
    fn host_axes_detect_a_real_variant() {
        // os/arch always resolve to one of the closed variants on any host.
        assert!(Os::ALL.contains(&Os::detect()));
        assert!(Arch::ALL.contains(&Arch::detect()));
        // The test host is not a Kubernetes pod.
        assert_eq!(Orchestrator::detect(), Orchestrator::Bare);
    }

    #[test]
    fn display_matches_as_str() {
        assert_eq!(Tenancy::Foreign.to_string(), "foreign");
        assert_eq!(CapacityType::OnDemand.to_string(), "on-demand");
    }
}
