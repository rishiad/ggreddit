#!/bin/sh
set -eu

usage() {
	cat <<'EOF'
Usage: scripts/setup.sh [--engine docker|podman] [--data-dir DIR]
                        [--config FILE] [--image IMAGE] [--name NAME]
                        [--port PORT] [--no-build]

Build and start ggreddit. The default config is copied to DATA_DIR/config.toml
only once. Custom configs must listen on 0.0.0.0:8080 and use the container
paths /run/ggreddit/media.key and /var/cache/ggreddit/cache.sqlite.
The default port binds to localhost; put a reverse proxy in front if needed.
Run as a non-root host user with access to the container engine.
EOF
}

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd -P)
engine=docker
data_dir="$root/data/container"
config=
image=ggreddit:local
name=ggreddit
port=8080
build=1

while [ "$#" -gt 0 ]; do
	case $1 in
		--engine|--data-dir|--config|--image|--name|--port)
			option=$1
			[ "$#" -ge 2 ] || { usage >&2; exit 2; }
			case $option in
				--engine) engine=$2 ;;
				--data-dir) data_dir=$2 ;;
				--config) config=$2 ;;
				--image) image=$2 ;;
				--name) name=$2 ;;
				--port) port=$2 ;;
			esac
			shift 2 ;;
		--no-build) build=0; shift ;;
		-h|--help) usage; exit 0 ;;
		*) usage >&2; exit 2 ;;
	esac
done

case $engine in docker|podman) ;; *) printf 'Unsupported engine: %s\n' "$engine" >&2; exit 2 ;; esac
case $port in ''|*[!0-9]*) printf 'Invalid port: %s\n' "$port" >&2; exit 2 ;; esac
[ "$port" -ge 1 ] && [ "$port" -le 65535 ] || { printf 'Port must be between 1 and 65535\n' >&2; exit 2; }
[ "$(id -u)" -ne 0 ] || { printf 'Run setup as a non-root user\n' >&2; exit 1; }
command -v "$engine" >/dev/null || { printf 'Container engine not found: %s\n' "$engine" >&2; exit 1; }

umask 077
mkdir -p "$data_dir/cache"
data_dir=$(CDPATH= cd -- "$data_dir" && pwd -P)
if [ -z "$config" ]; then
	config="$data_dir/config.toml"
	if [ ! -e "$config" ]; then
		cp "$root/config.container.toml" "$config"
	fi
else
	[ -f "$config" ] || { printf 'Config not found: %s\n' "$config" >&2; exit 1; }
	config=$(CDPATH= cd -- "$(dirname -- "$config")" && pwd -P)/$(basename -- "$config")
fi

for path in "$config" "$data_dir"; do
	case $path in *,*) printf 'Paths cannot contain commas: %s\n' "$path" >&2; exit 1 ;; esac
done
key="$data_dir/media.key"
"$root/scripts/media_key_gen.sh" "$key"

if [ "$build" -eq 1 ]; then
	"$engine" build -t "$image" -f "$root/Containerfile" "$root"
fi

"$engine" run --rm \
	--mount "type=bind,source=$config,target=/etc/ggreddit/config.toml,readonly" \
	"$image" --check-config

"$engine" run --detach --name "$name" --restart unless-stopped \
	--user "$(id -u):$(id -g)" \
	--publish "127.0.0.1:$port:8080" \
	--mount "type=bind,source=$config,target=/etc/ggreddit/config.toml,readonly" \
	--mount "type=bind,source=$key,target=/run/ggreddit/media.key,readonly" \
	--mount "type=bind,source=$data_dir/cache,target=/var/cache/ggreddit" \
	"$image"
