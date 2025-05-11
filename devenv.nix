{ pkgs, lib, config, inputs, ... }:

{
  # https://devenv.sh/packages/
  packages = [ pkgs.trunk pkgs.git pkgs.gdb pkgs.sqlx-cli pkgs.sql-formatter ];

  # https://devenv.sh/languages/
  languages.rust = {
    enable = true;
    channel = "stable";
    targets = ["wasm32-unknown-unknown"];
  };

  # See full reference at https://devenv.sh/reference/options/
}
