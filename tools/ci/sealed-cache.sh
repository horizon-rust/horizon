#!/usr/bin/env bash
# Game-derived folders sealed for a cache entry, and unsealed from one (.github/workflows/parity-hosted.yml).
#
#   VANILLA_DATA_PASSPHRASE=... bash tools/ci/sealed-cache.sh seal LABEL KEY ARCHIVE [--exclude PATH]... FOLDER...
#   VANILLA_DATA_PASSPHRASE=... bash tools/ci/sealed-cache.sh unseal LABEL PREFIX ARCHIVE [--exclude PATH]... STAGE
#
# A fork's pull request can restore the default branch's cache entries, and a branch's own copy of a workflow can
# name any key, so what is made from the vanilla data goes into a cache only as this archive:
# - a POSIX tar (nanosecond modification times: tools/parity_shim.py replays an oracle run only while its outputs
#   keep theirs), zstd, then the dataset's own encryption (openssl enc -aes-256-cbc -pbkdf2 -iter 600000 -md sha256,
#   the passphrase of horizon-rust/vanilla-data);
# - ARCHIVE.mac: the entry's cache KEY, then HMAC-SHA256 over a version, LABEL, KEY and the whole ciphertext, keyed by
#   PBKDF2 of the same passphrase with a salt of its own, so the two keys are independent. openssl enc has no
#   authenticated mode, and age reads a passphrase only from a terminal.
# unseal takes the KEY from ARCHIVE.mac only when it starts with PREFIX (the key this run computes, up to where it
# restores by prefix), and checks the tag before it decrypts anything: an entry someone without the passphrase wrote,
# one sealed under another LABEL, or one sealed for another key (an older dataset's under a newer key) is never
# decrypted. It then lists the archive, and unpacks it into STAGE (a new folder under RUNNER_TEMP, laid out as from
# /, which the workflow moves into place) only if every member is a file, folder or link inside the two folders
# below: no absolute or `..` name, no link leading out of them, no member under a link, no device, pipe or other
# special file. Whatever fails (no tag, a wrong tag or key, a broken archive, a refused member) is a warning: STAGE is
# removed and the run goes on cold, never fails for it. Nothing in an archive is ever run.
#
# FOLDERs are absolute paths under the checkout or RUNNER_TEMP. An excluded PATH (a file or folder in them, such as
# the oracle shim's machine-local runtime memo) is never sealed, and unseal removes it from the stage should an
# entry hold it. The passphrase comes from this step's environment, is
# written to a private temporary file and unset at once, and that file goes before the script ends, whatever happens.
# The MAC key never reaches a command line: the runner's own Python derives it from the file. Only sizes, counts and
# seconds are printed, never a file name or the secret.
set -euo pipefail
umask 077
mode=${1:?seal or unseal}
label=${2:?label}
key=${3:?key or prefix}
archive=${4:?archive}
shift 4
excluded=()
while [ "${1-}" = --exclude ]; do
  excluded+=("$(realpath -m "${2:?--exclude needs a path}")")
  shift 2
done
[[ "$label" =~ ^[a-z][a-z0-9-]*$ ]] || { echo "::error::bad label"; exit 2; }
[[ "$key" =~ ^[A-Za-z0-9._+-]{1,512}$ ]] || { echo "::error::bad key"; exit 2; }
: "${VANILLA_DATA_PASSPHRASE:?}" "${GITHUB_WORKSPACE:?}" "${RUNNER_TEMP:?}"
workspace=$(realpath -m "$GITHUB_WORKSPACE")
temp=$(realpath -m "$RUNNER_TEMP")
allowed() { case "$1/" in "$workspace"/?*|"$temp"/?*) return 0 ;; *) return 1 ;; esac; }
for path in "${excluded[@]}"; do allowed "$path" || { echo "::error::an excluded path must be under the checkout or RUNNER_TEMP"; exit 2; }; done

work=$(mktemp -d "$temp/sealed-cache.XXXXXX")
cleanup() { rm -rf "$work"; }
trap cleanup EXIT
printf '%s' "${VANILLA_DATA_PASSPHRASE//[$'\r\n']/}" > "$work/passphrase"
unset VANILLA_DATA_PASSPHRASE
cipher=(-aes-256-cbc -pbkdf2 -iter 600000 -md sha256 -pass "file:$work/passphrase")
started=$SECONDS

# mac seal FILE TAGS: write KEY and FILE's tag to TAGS. mac unseal FILE TAGS: exit 0 when TAGS names a key starting
# with PREFIX and holds FILE's tag under it (compared in constant time), 3 for another key, 1 for a wrong or missing
# tag. Isolated (-I) and run from the private folder: no module of the checkout or the current folder can stand in for
# hashlib or hmac.
mac() {
  (cd "$work" && /usr/bin/python3 -I - "$work/passphrase" "$label" "$key" "$@" <<'EOF'
import hashlib, hmac, re, sys
passphrase, label, key, mode, path, tags = sys.argv[1:]
if mode == "unseal":
    try:
        with open(tags, "rb") as file:
            stored, tag = (file.read(4096).decode("ascii").splitlines() + ["", ""])[:2]
    except (OSError, UnicodeDecodeError):
        sys.exit(1)
    if not re.fullmatch(r"[A-Za-z0-9._+-]{1,512}", stored) or not stored.startswith(key):
        sys.exit(3)
    key = stored
with open(passphrase, "rb") as file:
    secret = hashlib.pbkdf2_hmac("sha256", file.read(), b"horizon-rust/horizon sealed-cache MAC key v1", 600000)
mac = hmac.new(secret, b"horizon sealed-cache v2\0" + label.encode() + b"\0" + key.encode() + b"\0", "sha256")
with open(path, "rb") as file:
    for block in iter(lambda: file.read(1 << 20), b""):
        mac.update(block)
if mode == "seal":
    with open(tags, "w") as file:
        file.write(f"{key}\n{mac.hexdigest()}\n")
else:
    sys.exit(0 if hmac.compare_digest(tag.encode(), mac.hexdigest().encode()) else 1)
EOF
  )
}

# listed: read a tar on stdin; exit 0 only when every member may be unpacked (see above), 10 when one may not, 11
# when it is not a tar. Names stay inside: nothing is printed. The program is an argument: stdin is the tar.
read -r -d '' listing <<'EOF' || true
import posixpath, sys, tarfile
roots = sys.argv[1:]
def inside(name):
    return name == posixpath.normpath(name) and any(name == root or name.startswith(root + "/") for root in roots)
links, files, refused = set(), set(), 0
try:
    with tarfile.open(fileobj=sys.stdin.buffer, mode="r|", bufsize=1 << 20) as archive:
        for member in archive:
            name = member.name.rstrip("/") if member.isdir() else member.name
            parents = {name[:index] for index, char in enumerate(name) if char == "/"}
            ok = inside(name) and name not in links and not parents & links
            if member.issym():
                # The stage is laid out as from /: an absolute target is read from there, a relative one from the link.
                target = member.linkname
                target = posixpath.normpath(target.lstrip("/") if target.startswith("/")
                                            else posixpath.join(posixpath.dirname(name), target))
                ok = ok and inside(target)
                links.add(name)
            elif member.islnk():
                ok = ok and inside(member.linkname) and member.linkname in files
            elif member.isreg() and not member.issparse():
                files.add(name)
            elif not member.isdir():
                ok = False
            refused += not ok
except (tarfile.TarError, EOFError, OSError):
    sys.exit(11)
sys.exit(10 if refused else 0)
EOF
listed() { (cd "$work" && /usr/bin/python3 -I -c "$listing" "${workspace#/}" "${temp#/}"); }

case "$mode" in
  seal)
    members=()
    for folder in "$@"; do
      folder=$(realpath -m "$folder")
      allowed "$folder" || { echo "::error::a sealed folder must be under the checkout or RUNNER_TEMP"; exit 2; }
      [ -e "$folder" ] && members+=("${folder#/}")
    done
    [ "${#members[@]}" -gt 0 ] || { echo "nothing to seal"; exit 0; }
    mkdir -p "$(dirname "$archive")"
    # Member names are relative to / and anchored: an excluded folder is left out with everything under it.
    patterns=()
    for path in "${excluded[@]}"; do patterns+=("--exclude=${path#/}"); done
    tar -C / --format=posix --pax-option=delete=atime,delete=ctime --anchored "${patterns[@]}" \
      -cf - -- "${members[@]}" |
      zstd -q -T0 -3 |
      openssl enc -e "${cipher[@]}" > "$work/archive"
    mac seal "$work/archive" "$work/archive.mac"
    mv "$work/archive" "$archive"
    mv "$work/archive.mac" "$archive.mac"
    echo "sealed ${#members[@]} folders, $(du -sk "${members[@]/#//}" | awk '{s+=$1} END {print s}') KiB," \
      "into $(( $(stat -c %s "$archive") / 1024 )) KiB in $((SECONDS - started)) s"
    ;;
  unseal)
    stage=$(realpath -m "${1:?stage}")
    case "$stage/" in "$temp"/?*) ;; *) echo "::error::the stage must be under RUNNER_TEMP"; exit 2 ;; esac
    [ ! -e "$stage" ] || { echo "::error::the stage already exists"; exit 2; }
    discard() { echo "::warning::$label cache entry discarded ($1): this run starts without it"; rm -rf "$stage"; exit 0; }
    if [ ! -f "$archive" ] || [ -L "$archive" ]; then echo "no $label cache entry"; exit 0; fi
    if [ ! -f "$archive.mac" ] || [ -L "$archive.mac" ]; then discard "no authentication tag"; fi
    status=0
    mac unseal "$archive" "$archive.mac" || status=$?
    [ "$status" -ne 3 ] || discard "it was sealed for another key"
    [ "$status" -eq 0 ] || discard "its authentication tag does not match"
    # Decrypted once into a private file: the archive listed is the archive unpacked.
    openssl enc -d "${cipher[@]}" < "$archive" > "$work/inner.tar.zst" 2> /dev/null || discard "it did not decrypt"
    status=0
    zstd -dcq < "$work/inner.tar.zst" 2> /dev/null | listed || status=$?
    [ "$status" -ne 10 ] || discard "it holds a member that may not be unpacked"
    [ "$status" -eq 0 ] || discard "it did not unpack"
    mkdir -p "$stage"
    zstd -dcq < "$work/inner.tar.zst" 2> /dev/null | tar -C "$stage" -x --no-same-owner 2> /dev/null ||
      discard "it did not unpack"
    for path in "${excluded[@]}"; do
      if [ -e "$stage$path" ] || [ -L "$stage$path" ]; then
        rm -rf "${stage:?}$path"
        echo "::warning::the $label cache entry held an excluded path: removed before it is placed"
      fi
    done
    echo "unsealed $label: $(( $(stat -c %s "$archive") / 1024 )) KiB in $((SECONDS - started)) s"
    ;;
  *) echo "::error::seal or unseal"; exit 2 ;;
esac
