{
  description = "NixOS module and package for t2fanrd";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux"; # As this is for T2 Macs, only x86_64 needed.
      pkgs = nixpkgs.legacyPackages.${system};
    in {
      packages.${system}.default = pkgs.rustPlatform.buildRustPackage {
        pname = "t2fanrd";
        version = "0.2.0";
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;
      };
      nixosModules.t2fanrd = { config, lib, ... }:
        let
          cfg = config.services.t2fanrd;
          configFile = (pkgs.formats.toml { }).generate "t2fanrd.toml" cfg.settings;
        in {
          options.services.t2fanrd = {
            enable = lib.mkEnableOption "t2fanrd daemon to manage fan curves for T2 Macs";

            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${system}.default;
              description = "T2fanrd package to use.";
            };

            settings = lib.mkOption {
              type = (pkgs.formats.toml { }).type;
              default = builtins.fromTOML (builtins.readFile ./t2fanrd.toml);
              description = ''
                The contents of t2fanrd.toml: interval, stale_after, [output]
                and the [[curve]] list. See README.md. The default is the
                config for the 13-inch 2020 MacBook Pro.
              '';
            };
          };
          config = lib.mkIf cfg.enable {
            environment.etc."t2fanrd.toml".source = configFile;
            systemd.services.t2fanrd = {
              description = "T2FanRD daemon to manage the fans on a T2 Mac";
              wantedBy = [ "multi-user.target" ];
              restartTriggers = [ configFile ];
              serviceConfig = {
                Type = "exec";
                ExecStart = "${cfg.package}/bin/t2fanrd --config /etc/t2fanrd.toml";
                Restart = "always";

                # Taken from the fedora system service
                # https://github.com/t2linux/fedora/blob/2947fdc909a35f04eb936a4f9c0f33fe4e52d9c2/t2fanrd/t2fanrd.service
                PrivateTmp = "true";
                ProtectSystem = "true";
                ProtectHome = "true";
                ProtectClock = "true";
                ProtectControlGroups = "true";
                ProtectHostname = "true";
                ProtectKernelLogs = "true";
                ProtectKernelModules = "true";
                ProtectProc = "invisible";
                PrivateDevices = "true";
                PrivateNetwork = "true";
                NoNewPrivileges = "true";
                DevicePolicy = "closed";
                KeyringMode = "private";
                LockPersonality = "true";
                MemoryDenyWriteExecute = "true";
                PrivateUsers = "yes";
                RemoveIPC = "yes";
                RestrictNamespaces = "yes";
                RestrictRealtime = "yes";
                RestrictSUIDSGID = "yes";
                SystemCallArchitectures = "native";
              };
            };
          };
        };
    };
}
