{
  description = "Chibi - A CLI tool for conversing with LLMs via OpenRouter";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "chibi";
          version = "0.2.0";
          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          nativeBuildInputs = [
            pkgs.pkg-config
          ];

          buildInputs = [
            pkgs.openssl
          ];

          meta = {
            description = "CLI tool for conversing with LLMs via OpenRouter";
            homepage = "https://github.com/emesal/chibi";
            license = pkgs.lib.licenses.mit;
            maintainers = [ ];
            mainProgram = "chibi";
          };
        };

        devShells.default = pkgs.mkShell {
          inputsFrom = [ self.packages.${system}.default ];
          packages = [
            pkgs.cargo
            pkgs.rustc
            pkgs.rustfmt
            pkgs.clippy
          ];
        };
      }
    );
}
