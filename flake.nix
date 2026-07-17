{
  description = "Kanchi (感知) — typed environment-discovery primitive: declare detection axes, get FALLBACK const + detect()→Option + detect_or_fallback() generated";

  # substrate.rust.library dispatches over Cargo.gen.lock (the slim gen delta,
  # reconstructed to the full BuildSpec in pure Nix) — no crate2nix, no Cargo.nix.
  inputs.substrate.url = "github:pleme-io/substrate";

  outputs = { substrate, ... }: substrate.rust.library {
    src = ./.;
  };
}
