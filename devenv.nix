{ pkgs, lib, config, inputs, ... }:

{
  # https://devenv.sh/packages/
  packages = [ pkgs.git pkgs.sqlx-cli pkgs.sql-formatter ];

  # https://devenv.sh/languages/
  languages.rust.enable = true;

  # See full reference at https://devenv.sh/reference/options/
}
