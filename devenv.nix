{ pkgs, lib, config, inputs, ... }:

{
  # https://devenv.sh/packages/
  packages = [ pkgs.git pkgs.sqlx-cli pkgs.sqlformat ];

  # https://devenv.sh/languages/
  languages.rust.enable = true;

  # See full reference at https://devenv.sh/reference/options/
}
