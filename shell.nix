{
  mkShell,
  cargo,
  rustc,
  nodejs,
  tree-sitter,
}:
mkShell {
  nativeBuildInputs = [
    cargo
    rustc
    nodejs
    tree-sitter
  ];
}
