{
  description = "Client for the LG webOS TV SSAP API, plus a `tv` CLI";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = nixpkgs.legacyPackages.${system};
    in {
      packages.default = pkgs.rustPlatform.buildRustPackage {
        pname = "webos-ssap";
        version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
        src = self;
        cargoLock.lockFile = ./Cargo.lock;
        meta.mainProgram = "tv";
      };

      devShells.default = pkgs.mkShell {
        inputsFrom = [self.packages.${system}.default];
        packages = with pkgs; [clippy rustfmt rust-analyzer];
      };
    });
}
