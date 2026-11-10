{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.neddit;
  format = pkgs.formats.toml { };
  settings = lib.recursiveUpdate (lib.recursiveUpdate {
    cache.path = "/var/cache/neddit/cache.sqlite";
  } cfg.settings) (lib.optionalAttrs (cfg.mediaKeyFile != null) {
    media.signing_key_file = "/run/credentials/neddit.service/media-key";
  });
  configFile = format.generate "neddit.toml" settings;
in
{
  options.services.neddit = {
    enable = lib.mkEnableOption "Private Reddit frontend";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      description = "The Neddit package to run.";
    };

    settings = lib.mkOption {
      type = format.type;
      default = {
        server.listen = "127.0.0.1:8080";
      };
      description = "Neddit settings written to its TOML configuration file.";
    };

    mediaKeyFile = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "Path to a file containing at least 32 bytes used to sign media proxy URLs.";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.services.neddit = {
      description = "Neddit";
      wantedBy = [ "multi-user.target" ];
      wants = [ "network-online.target" ];
      after = [ "network-online.target" ];
      serviceConfig = {
        Type = "exec";
        ExecStart = "${cfg.package}/bin/neddit --config ${configFile}";
        Restart = "on-failure";
        RestartSec = 5;

        DynamicUser = true;
        StateDirectory = "neddit";
        CacheDirectory = "neddit";
        AmbientCapabilities = "";
        CapabilityBoundingSet = "";
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
        NoNewPrivileges = true;
        PrivateDevices = true;
        PrivateTmp = true;
        ProcSubset = "pid";
        ProtectClock = true;
        ProtectControlGroups = true;
        ProtectHome = true;
        ProtectHostname = true;
        ProtectKernelLogs = true;
        ProtectKernelModules = true;
        ProtectKernelTunables = true;
        ProtectProc = "invisible";
        ProtectSystem = "strict";
        RemoveIPC = true;
        RestrictAddressFamilies = [
          "AF_INET"
          "AF_INET6"
        ];
        RestrictNamespaces = true;
        RestrictRealtime = true;
        RestrictSUIDSGID = true;
        SystemCallArchitectures = "native";
        SystemCallFilter = [
          "@system-service"
          "~@privileged"
          "~@resources"
        ];
        UMask = "0077";
      } // lib.optionalAttrs (cfg.mediaKeyFile != null) {
        LoadCredential = "media-key:${cfg.mediaKeyFile}";
      };
    };
  };
}
