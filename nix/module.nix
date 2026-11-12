{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.ggreddit;
  format = pkgs.formats.toml { };
  settings = lib.recursiveUpdate (lib.recursiveUpdate {
    cache.path = "/var/cache/ggreddit/cache.sqlite";
  } cfg.settings) (lib.optionalAttrs (cfg.mediaKeyFile != null) {
    media.signing_key_file = "/run/credentials/ggreddit.service/media-key";
  });
  configFile = format.generate "ggreddit.toml" settings;
in
{
  options.services.ggreddit = {
    enable = lib.mkEnableOption "Private Reddit frontend";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      description = "The ggreddit package to run.";
    };

    settings = lib.mkOption {
      type = format.type;
      default = {
        server.listen = "127.0.0.1:8080";
      };
      description = "ggreddit settings written to its TOML configuration file. Set domains.media explicitly to proxy images and Reddit videos.";
    };

    mediaKeyFile = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "Path to a file containing at least 32 bytes used to sign media proxy URLs.";
    };
  };

  config = lib.mkIf cfg.enable {
    systemd.services.ggreddit = {
      description = "ggreddit";
      wantedBy = [ "multi-user.target" ];
      wants = [ "network-online.target" ];
      after = [ "network-online.target" ];
      serviceConfig = {
        Type = "exec";
        ExecStart = "${cfg.package}/bin/ggreddit --config ${configFile}";
        Restart = "on-failure";
        RestartSec = 5;

        DynamicUser = true;
        CacheDirectory = "ggreddit";
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
