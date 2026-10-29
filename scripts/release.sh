#!/usr/bin/env bash
# Cuts a release from dev, in the shape every release so far has had:
#
#   master  merges dev (--no-ff), takes the release version, dates the
#           CHANGELOG's [Unreleased] section, and is tagged vX.Y.Z;
#   dev     merges master back (--no-ff) and moves on to the next -dev version
#           under a fresh [Unreleased] section.
#
# Before running it, curate the [Unreleased] section on dev and push. Nothing
# is pushed until you confirm. Pushing the tag runs release.yml, which builds
# the binaries and opens a draft release whose notes come from the CHANGELOG;
# edit them on GitHub if need be, and publishing the draft runs publish.yml.
#
# Usage: scripts/release.sh [VERSION] [--next VERSION] [--no-edit]
#
#   VERSION     the version to release; defaults to Cargo.toml's without -dev
#   --next      the version dev moves on to, without -dev; defaults to the next
#               patch version
#   --no-edit   don't open the CHANGELOG in $EDITOR before the release commit
set -euo pipefail
cd "$(dirname "$0")/.."

die() { echo "release: $*" >&2; exit 1; }
confirm() { local answer; read -r -p "$1 [y/N] " answer; [[ $answer == [yY] ]]; }
is_version() { [[ $1 =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; }
# True when version $1 comes strictly after version $2.
version_gt() { [[ $1 != "$2" && $(printf '%s\n%s\n' "$1" "$2" | sort -V | tail -n1) == "$1" ]]; }

# Rewrites FILE through the awk program and variables given after it.
rewrite() {
    local file=$1; shift
    awk "$@" "$file" >"$file.new"
    mv "$file.new" "$file"
}

# Merges branch $1 into the current one and stops short of committing. Git's
# chatter is shown only if the merge fails.
merge() {
    local out
    out=$(git merge --no-ff --no-commit "$1" 2>&1) || { echo "$out" >&2; return 1; }
}

set_version() {
    rewrite Cargo.toml -v v="$1" '!done && /^version = "/ { $0 = "version = \"" v "\""; done = 1 } 1'
    cargo update --quiet --offline --workspace
}

version= next= edit=1
while (($#)); do
    case $1 in
        --next) (($# > 1)) || die "--next needs a version"; next=$2; shift 2 ;;
        --no-edit) edit=0; shift ;;
        -h | --help) sed -n '2,/^set /s/^# \{0,1\}//p' "$0"; exit 0 ;;
        -*) die "unknown option: $1" ;;
        *) [[ -z $version ]] || die "more than one version given"; version=$1; shift ;;
    esac
done

repo=$(sed -n 's/^repository = "\(.*\)"$/\1/p' Cargo.toml)
current=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n1)
[[ -n $repo && -n $current ]] || die "can't read version and repository from Cargo.toml"

# --- Versions -----------------------------------------------------------------

git fetch --quiet --tags origin
prev_tag=$(git tag --list 'v*.*.*' --sort=-v:refname | head -n1)
[[ -n $prev_tag ]] || die "no earlier vX.Y.Z tag to compare against"

version=${version:-${current%-dev}}
is_version "$version" || die "not a plain X.Y.Z version: $version"
version_gt "$version" "${prev_tag#v}" || die "$version does not come after the latest release, $prev_tag"
! git rev-parse --quiet --verify "refs/tags/v$version" >/dev/null || die "tag v$version already exists"

if [[ -z $next ]]; then
    IFS=. read -r major minor patch <<<"$version"
    next=$major.$minor.$((patch + 1))
fi
is_version "$next" || die "--next is not a plain X.Y.Z version: $next"
version_gt "$next" "$version" || die "--next $next does not come after $version"

# --- Repository state ---------------------------------------------------------

[[ $(git symbolic-ref --short --quiet HEAD) == dev ]] || die "not on dev"
[[ -z $(git status --porcelain) ]] || die "the working tree is not clean"
for branch in dev master; do
    [[ $(git rev-parse "$branch") == $(git rev-parse "origin/$branch") ]] ||
        die "$branch is not in sync with origin/$branch"
done
git merge-base --is-ancestor master dev || die "master has commits dev lacks; merge master into dev first"

grep -qx '## \[Unreleased\]' CHANGELOG.md || die "CHANGELOG.md has no '## [Unreleased]' section"
unreleased=$(awk '/^## \[/ { if (on) exit; on = ($0 == "## [Unreleased]"); next } on && NF' CHANGELOG.md)
[[ -n $unreleased && $unreleased != TODO ]] || die "CHANGELOG.md's [Unreleased] section is empty; write it on dev first"
unreleased_link="[Unreleased]: $repo/compare/$prev_tag...HEAD"
grep -qxF "$unreleased_link" CHANGELOG.md || die "CHANGELOG.md lacks the link line: $unreleased_link"

if command -v gh >/dev/null; then
    ci=$(gh run list --workflow ci.yml --branch dev --commit "$(git rev-parse dev)" --limit 1 \
        --json status,conclusion --jq '.[0] | "\(.status)/\(.conclusion)"' 2>/dev/null || true)
    if [[ $ci != completed/success ]]; then
        echo "release: CI for dev at $(git rev-parse --short dev) is ${ci:-not found}" >&2
        confirm "Release anyway?" || exit 1
    fi
else
    echo "release: gh is not installed, so CI on GitHub is not checked" >&2
fi

# The same checks as ci.yml. Their stdin is closed so that nothing they run can
# swallow the answers to the prompts below.
echo "Running the CI checks locally..."
cargo fmt --all --check </dev/null
cargo clippy --quiet --locked --all-targets -- -D warnings </dev/null
cargo test --quiet --locked </dev/null
./scripts/gen-manual.sh </dev/null
git diff --quiet MANUAL.md || die "MANUAL.md is stale; run scripts/gen-manual.sh and commit the result"

today=$(date +%F)
cat <<EOF

Release:      $version  (tag v$version on master, CHANGELOG dated $today)
Previous:     $prev_tag
Next on dev:  $next-dev

EOF
confirm "Create the release and post-release commits?" || exit 1

# --- Commits ------------------------------------------------------------------

# From here on, anything but a completed push leaves local changes behind:
# offer to undo them, or say how.
dev_start=$(git rev-parse --short dev)
master_start=$(git rev-parse --short master)
print_only=0
step() { if ((print_only)); then echo "  $*"; else "$@"; fi; }

# Puts dev, master and the tags back as they were, doing only what is needed.
undo() {
    local on_dev=0 dirty=0
    if [[ $(git symbolic-ref --short --quiet HEAD) == dev ]]; then
        on_dev=1
        if [[ -n $(git status --porcelain) ]] || git rev-parse --quiet --verify MERGE_HEAD >/dev/null; then
            dirty=1
        fi
    fi
    # Leaving master also discards a merge or edits left unfinished there.
    ((on_dev)) || step git checkout --force dev
    if ((dirty)) || [[ $(git rev-parse --short dev) != "$dev_start" ]]; then
        step git reset --hard "$dev_start"
    fi
    [[ $(git rev-parse --short master) == "$master_start" ]] || step git branch --force master "$master_start"
    ! git rev-parse --quiet --verify "refs/tags/v$version" >/dev/null || step git tag --delete "v$version"
}

on_exit() {
    (($1 == 0)) && return
    echo >&2
    if confirm "Undo the changes this script made locally?"; then
        undo
        echo "Undone: dev, master and the tags are as they were." >&2
    else
        echo "Left as they are. To undo them later:" >&2
        print_only=1 undo >&2
    fi
}
trap 'on_exit $?' EXIT

# Neither merge can conflict: master is an ancestor of dev, checked above.
git checkout --quiet master
merge dev
set_version "$version"
rewrite CHANGELOG.md \
    -v heading="## [$version] - $today" \
    -v old="$unreleased_link" \
    -v new="[$version]: $repo/compare/$prev_tag...v$version" \
    '$0 == "## [Unreleased]" { $0 = heading } $0 == old { $0 = new } 1'
if ((edit)); then
    "${VISUAL:-${EDITOR:-vi}}" CHANGELOG.md || die "the editor exited with an error, so the release stops here"
fi
grep -q "^## \[$version\]" CHANGELOG.md || die "CHANGELOG.md lost its '## [$version]' heading"
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit --quiet -m "Release $version."
git tag -a "v$version" -m "Release $version."

git checkout --quiet dev
merge master
set_version "$next-dev"
rewrite CHANGELOG.md \
    -v heading="## [$version]" \
    -v link="[$version]: " \
    -v unreleased_link="[Unreleased]: $repo/compare/v$version...HEAD" \
    'index($0, heading) == 1 { print "## [Unreleased]\n\nTODO\n" }
     index($0, link) == 1 { print unreleased_link }
     1'
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit --quiet -m "Post release changes."

# --- Push ---------------------------------------------------------------------

echo
git log --oneline --graph -6 master dev
echo
if ! confirm "Push master, dev and v$version to origin?"; then
    echo "Not pushed. To push later: git push --atomic origin master dev v$version"
    exit 1
fi
git push --atomic origin master dev "v$version"
echo
echo "Pushed. release.yml now builds v$version and opens a draft release:"
echo "  $repo/releases"
echo "Review its notes there, then publish it to run publish.yml (crates.io)."
