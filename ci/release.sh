#!/usr/bin/env bash
# Release steps for .gitlab-ci.yml. Run from the repository root.
#
#   ci/release.sh check                Cargo.toml, CHANGELOG.md and the tag agree
#   ci/release.sh publish [--dry-run]  upload dist/* and SHA256SUMS to the generic
#                                      package registry and create the release
#
# publish is safe to re-run: a file already in the registry is skipped if it
# is identical and an error if not, and an existing release only gets its
# name and notes updated.
set -euo pipefail

FILES=(
    shellsync-x86_64-unknown-linux-musl
    shellsync-aarch64-apple-darwin
)

die() {
    echo "release: $*" >&2
    exit 1
}

version() {
    local id
    id=$(cargo pkgid)
    echo "${id##*[#@]}"
}

# The CHANGELOG.md section for version $1, without its heading.
notes() {
    awk -v heading="## [$1]" '
        /^## / { if (found) exit; found = index($0, heading) == 1; next }
        /^\[[^]]*\]: / { next }
        found
    ' CHANGELOG.md
}

check() {
    local version heading
    version=$(version)
    if [[ -n ${CI_COMMIT_TAG:-} ]]; then
        [[ $CI_COMMIT_TAG == "v$version" ]] ||
            die "tag $CI_COMMIT_TAG doesn't match version $version in Cargo.toml"
        [[ ${CI_COMMIT_REF_PROTECTED:-} == true ]] ||
            die "tag $CI_COMMIT_TAG isn't protected; protect v* under Settings > Repository > Protected tags"
    fi
    heading=$(grep -m1 -F "## [$version]" CHANGELOG.md) ||
        die "CHANGELOG.md has no section for $version"
    [[ $heading == "## [$version] - "[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9] ]] ||
        die "CHANGELOG.md: expected '## [$version] - YYYY-MM-DD', found '$heading'"
    [[ -n $(notes "$version" | tr -d '[:space:]') ]] ||
        die "CHANGELOG.md: the section for $version is empty"
    echo "release: v$version is ready"
}

api() {
    curl -sS --header "JOB-TOKEN: $CI_JOB_TOKEN" "$@"
}

# Uploads file $1 to URL $2, unless the same file is there already.
upload() {
    local file=$1 url=$2 existing status
    if [[ -n $DRY_RUN ]]; then
        echo "would upload $file to $url"
        return
    fi
    existing=$(mktemp)
    status=$(api -o "$existing" -w '%{http_code}' "$url")
    case $status in
        200)
            cmp -s "$file" "$existing" ||
                die "$url exists with different contents; a published file is never replaced"
            echo "$file: already uploaded"
            ;;
        404)
            api --fail-with-body --upload-file "$file" "$url" >/dev/null
            echo "$file: uploaded"
            ;;
        *) die "GET $url: HTTP $status" ;;
    esac
    rm -f "$existing"
}

# Creates release $1 from the JSON in file $2, or updates its name and notes.
release() {
    local tag=$1 json=$2 releases status update
    if [[ -n $DRY_RUN ]]; then
        echo "would create release $tag:"
        cat "$json"
        return
    fi
    releases=$CI_API_V4_URL/projects/$CI_PROJECT_ID/releases
    status=$(api -o /dev/null -w '%{http_code}' "$releases/$tag")
    case $status in
        200)
            update=$(mktemp)
            jq '{name, description}' "$json" >"$update"
            api --fail-with-body -X PUT -H 'Content-Type: application/json' \
                --data-binary "@$update" "$releases/$tag" >/dev/null
            echo "release $tag: name and notes updated"
            ;;
        404)
            api --fail-with-body -H 'Content-Type: application/json' \
                --data-binary "@$json" "$releases" >/dev/null
            echo "release $tag: created"
            ;;
        *) die "GET $releases/$tag: HTTP $status" ;;
    esac
}

publish() {
    local version tag packages notes json links
    check
    version=$(version)
    tag=v$version
    if [[ -z $DRY_RUN ]]; then
        [[ ${CI_COMMIT_TAG:-} == "$tag" ]] || die "publish runs in a pipeline for tag $tag only; use --dry-run"
        : "${CI_API_V4_URL:?}" "${CI_PROJECT_ID:?}" "${CI_JOB_TOKEN:?}"
    fi
    packages=${CI_API_V4_URL:-\$CI_API_V4_URL}/projects/${CI_PROJECT_ID:-\$CI_PROJECT_ID}/packages/generic/shellsync/$version
    notes=$(mktemp)
    json=$(mktemp)
    notes "$version" >"$notes"

    cd dist
    for f in "${FILES[@]}"; do
        [[ -f $f ]] || die "dist/$f is missing"
    done
    sha256sum "${FILES[@]}" >SHA256SUMS
    cat SHA256SUMS
    for f in "${FILES[@]}" SHA256SUMS; do
        upload "$f" "$packages/$f"
    done

    links=$(jq -n --arg base "$packages" '$ARGS.positional | map({
        name: .,
        url: "\($base)/\(.)",
        direct_asset_path: "/\(.)",
        link_type: (if . == "SHA256SUMS" then "other" else "package" end)
    })' --args "${FILES[@]}" SHA256SUMS)
    jq -n --arg tag "$tag" --arg name "shellsync $version" --rawfile notes "$notes" \
        --argjson links "$links" \
        '{tag_name: $tag, name: $name, description: $notes, assets: {links: $links}}' >"$json"
    release "$tag" "$json"
    rm -f "$notes" "$json"
}

DRY_RUN=
case ${1:-} in
    check) check ;;
    publish)
        case ${2:-} in
            "") ;;
            --dry-run) DRY_RUN=1 ;;
            *) die "unknown option: $2" ;;
        esac
        publish
        ;;
    *) die "usage: ci/release.sh check | publish [--dry-run]" ;;
esac
