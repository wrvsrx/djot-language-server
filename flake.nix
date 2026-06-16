{
  description = "djot-language-server";

  inputs = {
    nur-wrvsrx.url = "github:wrvsrx/nur-packages";
    nixpkgs.follows = "nur-wrvsrx/nixpkgs";
    flake-parts.follows = "nur-wrvsrx/flake-parts";
  };

  outputs =
    inputs:
    inputs.flake-parts.lib.mkFlake { inherit inputs; } (
      { inputs, ... }:
      {
        systems = [ "x86_64-linux" ];
        perSystem =
          { pkgs, ... }:
          {
            packages.default = pkgs.callPackage ./default.nix { };
            devShells.default = pkgs.callPackage ./shell.nix { };
            formatter = pkgs.nixfmt-rfc-style;
          };
      }
    );
}
