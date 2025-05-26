{
  pkgs,
  lib,
  config,
  inputs,
  ...
}: {
  # https://devenv.sh/packages/
  packages = with pkgs;[
    git
    gdb
    
    # ui
    slint-lsp
    qt6.qtbase
    qt6.qtsvg
    libGL

    # sql(x) stuffs
    sqlx-cli
    sql-formatter
    sqlite
  ];

  # https://devenv.sh/languages/
  languages.rust = {
    enable = true;
    channel = "stable";
  };

  # See full reference at https://devenv.sh/reference/options/
}
