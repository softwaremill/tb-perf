#!/usr/bin/env bash
#
# Downloads the exact Zig version that `tigerbeetle-unofficial-sys` needs to
# build TigerBeetle's C client, and exports ZIG_PATH so `cargo zigbuild` uses
# it instead of downloading its own copy.
#
# This exists because TigerBeetle's bundled zig/download.sh fetches Zig from
# pkg.machengine.org, which now redirects to a dead pkg.hexops.org host. We
# pull the identical archive (same checksum) from ziglang.org instead.
#
# Source it so ZIG_PATH lands in your shell (bash or zsh):
#     source scripts/setup-zig.sh
# or capture the path when running it directly:
#     export ZIG_PATH="$(scripts/setup-zig.sh)"

ZIG_VERSION="0.14.1"

_tb_zig_sourced=0
if [ -n "${ZSH_VERSION:-}" ]; then
    case "${ZSH_EVAL_CONTEXT:-}" in *:file*) _tb_zig_sourced=1 ;; esac
    eval '_tb_zig_self="${(%):-%x}"'
elif [ -n "${BASH_VERSION:-}" ]; then
    [ "${BASH_SOURCE[0]}" != "${0}" ] && _tb_zig_sourced=1
    _tb_zig_self="${BASH_SOURCE[0]}"
else
    _tb_zig_self="${0}"
fi

_tb_setup_zig() {
    local self_dir repo_root arch os archive url expected_sha cache_dir zig_dir

    self_dir="$(cd "$(dirname "${_tb_zig_self}")" && pwd)"
    repo_root="$(cd "${self_dir}/.." && pwd)"

    case "$(uname -m)" in
        arm64 | aarch64) arch="aarch64" ;;
        x86_64) arch="x86_64" ;;
        *) echo "Unsupported architecture: $(uname -m)" >&2; return 1 ;;
    esac

    case "$(uname)" in
        Linux) os="linux" ;;
        Darwin) os="macos" ;;
        *) echo "Unsupported OS: $(uname)" >&2; return 1 ;;
    esac

    case "${arch}-${os}" in
        aarch64-linux) expected_sha="f7a654acc967864f7a050ddacfaa778c7504a0eca8d2b678839c21eea47c992b" ;;
        aarch64-macos) expected_sha="39f3dc5e79c22088ce878edc821dedb4ca5a1cd9f5ef915e9b3cc3053e8faefa" ;;
        x86_64-linux) expected_sha="24aeeec8af16c381934a6cd7d95c807a8cb2cf7df9fa40d359aa884195c4716c" ;;
        x86_64-macos) expected_sha="b0f8bdfb9035783db58dd6c19d7dea89892acc3814421853e5752fe4573e5f43" ;;
        *) echo "No known Zig ${ZIG_VERSION} checksum for ${arch}-${os}" >&2; return 1 ;;
    esac

    archive="zig-${arch}-${os}-${ZIG_VERSION}.tar.xz"
    url="https://ziglang.org/download/${ZIG_VERSION}/${archive}"
    cache_dir="${repo_root}/.zig"
    zig_dir="${cache_dir}/zig-${arch}-${os}-${ZIG_VERSION}"

    if [ -x "${zig_dir}/zig" ] && [ "$("${zig_dir}/zig" version 2>/dev/null)" = "${ZIG_VERSION}" ]; then
        echo "Zig ${ZIG_VERSION} already present at ${zig_dir}" >&2
        echo "${zig_dir}/zig"
        return 0
    fi

    mkdir -p "${cache_dir}"

    echo "Downloading Zig ${ZIG_VERSION} from ${url} ..." >&2
    if ! curl --location --silent --show-error --fail --max-time 300 \
        --output "${cache_dir}/${archive}" "${url}"; then
        echo "Failed to download Zig from ${url}" >&2
        return 1
    fi

    local actual_sha
    if command -v sha256sum > /dev/null; then
        actual_sha="$(sha256sum "${cache_dir}/${archive}" | cut -d ' ' -f 1)"
    else
        actual_sha="$(shasum -a 256 "${cache_dir}/${archive}" | cut -d ' ' -f 1)"
    fi

    if [ "${actual_sha}" != "${expected_sha}" ]; then
        echo "Checksum mismatch for ${archive}" >&2
        echo "  expected: ${expected_sha}" >&2
        echo "  actual:   ${actual_sha}" >&2
        rm -f "${cache_dir}/${archive}"
        return 1
    fi

    echo "Extracting ${archive} ..." >&2
    rm -rf "${zig_dir}"
    if ! tar -xf "${cache_dir}/${archive}" -C "${cache_dir}"; then
        echo "Failed to extract ${archive}" >&2
        return 1
    fi
    rm -f "${cache_dir}/${archive}"

    if [ ! -x "${zig_dir}/zig" ]; then
        echo "Expected zig binary not found at ${zig_dir}/zig after extraction" >&2
        return 1
    fi

    echo "Zig ${ZIG_VERSION} ready at ${zig_dir}" >&2
    echo "${zig_dir}/zig"
}

_tb_zig_path="$(_tb_setup_zig)"
_tb_zig_status=$?
unset -f _tb_setup_zig

if [ "${_tb_zig_status}" -ne 0 ]; then
    unset _tb_zig_path _tb_zig_status _tb_zig_self
    if [ "${_tb_zig_sourced}" -eq 1 ]; then
        unset _tb_zig_sourced
        return 1
    fi
    unset _tb_zig_sourced
    exit 1
fi

if [ "${_tb_zig_sourced}" -eq 1 ]; then
    export ZIG_PATH="${_tb_zig_path}"
    echo "ZIG_PATH set to ${ZIG_PATH}" >&2
else
    echo "${_tb_zig_path}"
    echo "Set it with: export ZIG_PATH=\"${_tb_zig_path}\"" >&2
fi

unset _tb_zig_path _tb_zig_status _tb_zig_self _tb_zig_sourced
