//! Cluster-wiring discovery — the ConfigPlane **DISCOVERED tier**'s reach into
//! the *running* cluster.
//!
//! Where [`crate::probe`] answers "what host am I on?" (screen, RAM, cgroup
//! ceiling) and [`crate::axes`] classifies the environment (OS, arch,
//! orchestrator, cloud), this module answers the ConfigPlane question: **"what
//! is the cluster around me wired to?"** — the pod's own namespace/name via the
//! Kubernetes downward API, whether a named `Service` resolves through cluster
//! DNS, and (behind a mockable seam) the value of a `Secret` key.
//!
//! Three probe classes, split by what they cost:
//!
//! * **downward-API** ([`pod_namespace`], [`pod_name`]) — pure `std` env reads
//!   (the chart injects `POD_NAMESPACE` / `POD_NAME` via
//!   `fieldRef: metadata.{namespace,name}`), with the projected
//!   service-account `namespace` file as the namespace fallback. No client, no
//!   network.
//! * **service-DNS** ([`ClusterServiceDns`] + [`resolve_service`]) — a typed
//!   `<svc>.<ns>.svc.<cluster-domain>` FQDN resolved through `std`'s own
//!   resolver ([`std::net::ToSocketAddrs`]). Resolvable ⇒ the service exists in
//!   this cluster; unresolvable ⇒ [`None`], the clean degenerate.
//! * **Secret reads** — deliberately **behind the [`ClusterEnv`] trait seam**,
//!   because a real read needs a Kubernetes API client (SA token + CA cert +
//!   HTTPS). The `std`-only [`HostClusterEnv`] answers the first two classes for
//!   real and returns [`None`] for [`ClusterEnv::read_secret`] (no API client in
//!   the lean detection leaf); a cluster-aware consumer injects a real reader
//!   (e.g. shikumi's `kube`-feature reader) and tests inject [`MockClusterEnv`].
//!
//! **Totality / no-guess discipline.** Every probe returns [`Option`] and every
//! undetectable answer is [`None`] — never a fabricated default. An off-cluster
//! host lands `None` on every method here, which a shikumi `DiscoveryLayer`
//! turns into an *empty* contribution (the next config tier shows through),
//! exactly as [`crate::probe`]'s host probes degenerate to a documented
//! fallback.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;

/// The projected service-account namespace file every in-cluster pod carries —
/// the downward-API-independent fallback for [`pod_namespace`].
const SA_NAMESPACE_FILE: &str = "/var/run/secrets/kubernetes.io/serviceaccount/namespace";

/// The pod's own namespace, from the downward-API `POD_NAMESPACE` env var the
/// chart injects (`fieldRef: metadata.namespace`), falling back to the
/// projected service-account `namespace` file.
///
/// [`None`] off-cluster (neither the env var nor the SA file present) — the
/// clean degenerate, never a guess.
#[must_use]
pub fn pod_namespace() -> Option<String> {
    if let Some(ns) = non_empty_env("POD_NAMESPACE") {
        return Some(ns);
    }
    std::fs::read_to_string(SA_NAMESPACE_FILE)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// The pod's own name, from the downward-API `POD_NAME` env var the chart
/// injects (`fieldRef: metadata.name`). [`None`] when unset.
#[must_use]
pub fn pod_name() -> Option<String> {
    non_empty_env("POD_NAME")
}

/// A non-empty environment variable, or [`None`]. An env var set to the empty
/// string is treated as absent (a chart that injects an empty downward-API
/// value has told us nothing).
fn non_empty_env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

/// A typed in-cluster `Service` DNS name — `<service>.<namespace>.svc.<domain>`.
///
/// The FQDN is emitted through the [`std::fmt::Display`] impl (the one typed
/// surface for the name), so the dotted form is never open-coded with
/// `format!()` at a call site. Default cluster domain is `cluster.local`;
/// [`Self::with_cluster_domain`] overrides it for clusters configured
/// otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClusterServiceDns {
    service: String,
    namespace: String,
    cluster_domain: String,
}

impl ClusterServiceDns {
    /// The default Kubernetes cluster DNS domain.
    pub const DEFAULT_CLUSTER_DOMAIN: &'static str = "cluster.local";

    /// A `<service>.<namespace>.svc.cluster.local` name.
    #[must_use]
    pub fn new(service: impl Into<String>, namespace: impl Into<String>) -> Self {
        Self {
            service: service.into(),
            namespace: namespace.into(),
            cluster_domain: Self::DEFAULT_CLUSTER_DOMAIN.to_owned(),
        }
    }

    /// Override the cluster DNS domain (default `cluster.local`).
    #[must_use]
    pub fn with_cluster_domain(mut self, domain: impl Into<String>) -> Self {
        self.cluster_domain = domain.into();
        self
    }

    /// The service (short) name.
    #[must_use]
    pub fn service(&self) -> &str {
        &self.service
    }

    /// The namespace.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// The cluster DNS domain.
    #[must_use]
    pub fn cluster_domain(&self) -> &str {
        &self.cluster_domain
    }

    /// The fully-qualified `<service>.<namespace>.svc.<domain>` name as a
    /// [`String`] — the [`Display`][std::fmt::Display] rendering materialized.
    #[must_use]
    pub fn fqdn(&self) -> String {
        self.to_string()
    }
}

impl std::fmt::Display for ClusterServiceDns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}.{}.svc.{}",
            self.service, self.namespace, self.cluster_domain
        )
    }
}

/// Resolve a `host:port` through `std`'s DNS resolver — the in-cluster
/// service-DNS probe. [`Some`] with the first resolved [`SocketAddr`] means the
/// name resolves in this cluster (the `Service` exists / is wired); [`None`]
/// means it does not (off-cluster, or no such service) — the clean degenerate.
///
/// This is DNS resolution only — it proves the cluster-DNS record exists, not
/// that a backend is accepting connections. For an actual connect probe use
/// [`service_reachable`].
#[must_use]
pub fn resolve_service(host: &str, port: u16) -> Option<SocketAddr> {
    (host, port).to_socket_addrs().ok()?.next()
}

/// Whether a `host:port` accepts a TCP connection within `timeout` — a
/// stronger "is it actually reachable?" probe than [`resolve_service`]'s
/// DNS-only check. [`None`] when the name does not resolve; `Some(false)` when
/// it resolves but no connection completes in time.
#[must_use]
pub fn service_reachable(host: &str, port: u16, timeout: Duration) -> Option<bool> {
    let addr = resolve_service(host, port)?;
    Some(std::net::TcpStream::connect_timeout(&addr, timeout).is_ok())
}

/// The **Environment seam** for cluster-wiring discovery — the injectable trait
/// a shikumi `KubeClusterDiscovery` layer reads through, so a real read
/// (env / DNS / Kubernetes Secret API) is swapped for [`MockClusterEnv`] in
/// tests with no cluster in the loop.
///
/// Every method returns [`Option`]; the undetectable answer is always [`None`],
/// never a guess — so a layer built on this seam degenerates to an empty
/// contribution off-cluster.
pub trait ClusterEnv {
    /// The pod's own namespace (downward-API `POD_NAMESPACE` / SA file).
    fn namespace(&self) -> Option<String>;

    /// The pod's own name (downward-API `POD_NAME`).
    fn pod_name(&self) -> Option<String>;

    /// Resolve a [`ClusterServiceDns`] at `port` through cluster DNS.
    /// [`Some`] ⇒ the service resolves; [`None`] ⇒ it does not.
    fn resolve_service(&self, dns: &ClusterServiceDns, port: u16) -> Option<SocketAddr>;

    /// Read one `key` out of the named Kubernetes `Secret`. The one method that
    /// needs a real API client; [`None`] when unavailable / not found — never a
    /// fabricated value.
    fn read_secret(&self, secret: &str, key: &str) -> Option<String>;
}

/// The `std`-only, host-backed [`ClusterEnv`]: downward-API env reads + real
/// cluster-DNS resolution, and **[`None`] for every Secret read** (the lean
/// detection leaf carries no Kubernetes API client — a cluster-aware consumer
/// injects a real reader, e.g. shikumi's `kube`-feature reader).
///
/// This is the honest default: everything answerable without a client is
/// answered for real; the Secret read that *needs* a client degenerates to
/// [`None`] rather than guessing.
#[derive(Debug, Clone, Copy, Default)]
pub struct HostClusterEnv;

impl ClusterEnv for HostClusterEnv {
    fn namespace(&self) -> Option<String> {
        pod_namespace()
    }

    fn pod_name(&self) -> Option<String> {
        pod_name()
    }

    fn resolve_service(&self, dns: &ClusterServiceDns, port: u16) -> Option<SocketAddr> {
        resolve_service(&dns.fqdn(), port)
    }

    fn read_secret(&self, _secret: &str, _key: &str) -> Option<String> {
        // No Kubernetes API client in the detection leaf. A cluster-aware
        // consumer injects a real reader; here, None is the clean degenerate.
        None
    }
}

/// An in-memory [`ClusterEnv`] double for tests + shadow/dry-run: every answer
/// is whatever was registered, and anything unregistered is [`None`]. Lets a
/// shikumi `KubeClusterDiscovery` layer's whole logic be exercised with **no
/// cluster and no network**.
#[derive(Debug, Clone, Default)]
pub struct MockClusterEnv {
    namespace: Option<String>,
    pod_name: Option<String>,
    /// FQDNs that "resolve" (keyed by [`ClusterServiceDns::fqdn`]).
    resolvable: BTreeSet<String>,
    /// `(secret, key) -> value`.
    secrets: BTreeMap<(String, String), String>,
}

impl MockClusterEnv {
    /// An empty mock — every method returns [`None`] (the off-cluster shape).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register the pod namespace [`Self::namespace`] reports.
    #[must_use]
    pub fn with_namespace(mut self, ns: impl Into<String>) -> Self {
        self.namespace = Some(ns.into());
        self
    }

    /// Register the pod name [`Self::pod_name`] reports.
    #[must_use]
    pub fn with_pod_name(mut self, name: impl Into<String>) -> Self {
        self.pod_name = Some(name.into());
        self
    }

    /// Register a [`ClusterServiceDns`] as resolvable — [`Self::resolve_service`]
    /// returns a synthetic loopback [`SocketAddr`] for it (the layer only needs
    /// the Some/None witness + the FQDN).
    #[must_use]
    pub fn with_resolvable(mut self, dns: &ClusterServiceDns) -> Self {
        self.resolvable.insert(dns.fqdn());
        self
    }

    /// Register a Secret key value [`Self::read_secret`] returns.
    #[must_use]
    pub fn with_secret(
        mut self,
        secret: impl Into<String>,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        self.secrets.insert((secret.into(), key.into()), value.into());
        self
    }
}

impl ClusterEnv for MockClusterEnv {
    fn namespace(&self) -> Option<String> {
        self.namespace.clone()
    }

    fn pod_name(&self) -> Option<String> {
        self.pod_name.clone()
    }

    fn resolve_service(&self, dns: &ClusterServiceDns, port: u16) -> Option<SocketAddr> {
        self.resolvable
            .contains(&dns.fqdn())
            .then(|| SocketAddr::from(([127, 0, 0, 1], port)))
    }

    fn read_secret(&self, secret: &str, key: &str) -> Option<String> {
        self.secrets
            .get(&(secret.to_owned(), key.to_owned()))
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── downward-API ────────────────────────────────────────────────

    #[test]
    fn pod_namespace_reads_the_downward_api_env() {
        let key = "POD_NAMESPACE";
        // SAFETY: single-threaded test; restored below.
        unsafe { std::env::set_var(key, "camelot-dev") };
        assert_eq!(pod_namespace().as_deref(), Some("camelot-dev"));
        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn pod_name_reads_the_downward_api_env() {
        let key = "POD_NAME";
        unsafe { std::env::set_var(key, "auth-7c9f-abcde") };
        assert_eq!(pod_name().as_deref(), Some("auth-7c9f-abcde"));
        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn empty_downward_api_env_is_treated_as_absent() {
        let key = "POD_NAME";
        unsafe { std::env::set_var(key, "") };
        assert_eq!(pod_name(), None, "empty env value tells us nothing");
        unsafe { std::env::remove_var(key) };
    }

    // ── service DNS name (typed emission) ────────────────────────────

    #[test]
    fn service_dns_renders_the_fqdn() {
        let dns = ClusterServiceDns::new("mysql", "prod");
        assert_eq!(dns.fqdn(), "mysql.prod.svc.cluster.local");
        assert_eq!(dns.to_string(), "mysql.prod.svc.cluster.local");
    }

    #[test]
    fn service_dns_honors_a_custom_cluster_domain() {
        let dns = ClusterServiceDns::new("auth", "akeyless").with_cluster_domain("internal.example");
        assert_eq!(dns.fqdn(), "auth.akeyless.svc.internal.example");
        assert_eq!(dns.service(), "auth");
        assert_eq!(dns.namespace(), "akeyless");
        assert_eq!(dns.cluster_domain(), "internal.example");
    }

    // ── real DNS resolution: the clean degenerate off-cluster ────────

    #[test]
    fn resolve_service_returns_none_for_an_unresolvable_name() {
        // RFC 6761: `.invalid` is guaranteed never to resolve — a deterministic
        // NXDOMAIN, so the off-cluster degenerate is None, never a guess.
        assert_eq!(resolve_service("nonexistent-svc.doesnotexist.invalid", 80), None);
    }

    #[test]
    fn resolve_service_resolves_localhost() {
        // A name that always resolves proves the Some path is real, not stubbed.
        let addr = resolve_service("localhost", 80);
        assert!(addr.is_some(), "localhost must resolve on any host");
    }

    // ── HostClusterEnv: real for env/DNS, None for secrets ───────────

    #[test]
    fn host_env_reads_downward_api_and_never_guesses_secrets() {
        let key = "POD_NAMESPACE";
        unsafe { std::env::set_var(key, "prod") };
        let host = HostClusterEnv;
        assert_eq!(host.namespace().as_deref(), Some("prod"));
        // No API client in the leaf ⇒ every secret read is None, never a guess.
        assert_eq!(host.read_secret("db-creds", "password"), None);
        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn host_env_service_resolution_degenerates_to_none_off_cluster() {
        let host = HostClusterEnv;
        let dns = ClusterServiceDns::new("mysql", "doesnotexist").with_cluster_domain("invalid");
        assert_eq!(host.resolve_service(&dns, 3306), None);
    }

    // ── MockClusterEnv: full seam exercised with no cluster ──────────

    #[test]
    fn mock_env_answers_only_what_was_registered() {
        let db = ClusterServiceDns::new("mysql", "prod");
        let cache = ClusterServiceDns::new("redis", "prod");
        let env = MockClusterEnv::new()
            .with_namespace("prod")
            .with_pod_name("auth-0")
            .with_resolvable(&db)
            .with_secret("db-creds", "password", "s3cr3t");

        assert_eq!(env.namespace().as_deref(), Some("prod"));
        assert_eq!(env.pod_name().as_deref(), Some("auth-0"));
        assert!(env.resolve_service(&db, 3306).is_some(), "registered ⇒ resolves");
        assert_eq!(
            env.resolve_service(&cache, 6379),
            None,
            "unregistered ⇒ None, never a guess"
        );
        assert_eq!(env.read_secret("db-creds", "password").as_deref(), Some("s3cr3t"));
        assert_eq!(env.read_secret("db-creds", "username"), None);
        assert_eq!(env.read_secret("other", "password"), None);
    }

    #[test]
    fn empty_mock_is_the_off_cluster_shape() {
        let env = MockClusterEnv::new();
        let dns = ClusterServiceDns::new("svc", "ns");
        assert_eq!(env.namespace(), None);
        assert_eq!(env.pod_name(), None);
        assert_eq!(env.resolve_service(&dns, 80), None);
        assert_eq!(env.read_secret("s", "k"), None);
    }
}
