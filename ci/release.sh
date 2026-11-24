#!/usr/bin/env bash
# Release steps for .gitlab-ci.yml. Run from the repository root.
#
#   ci/release.sh check                Cargo.toml, CHANGELOG.md and the tag agree
#   ci/release.sh publish [--dry-run]  upload dist/* and SHA256SUMS to the generic
#                                      package registry and create the release
#   ci/release.sh github [--dry-run]   the same for the release on the GitHub
#                                      mirror ($GITHUB_REPOSITORY, $GITHUB_TOKEN)
#
# Both are safe to re-run: a file already uploaded is skipped if it is
# identical and an error if not, and an existing release only gets its name
# and notes updated.
set -euo pipefail

FILES=(
    shellsync-x86_64-unknown-linux-musl
    shellsync-aarch64-apple-darwin
    shellsync-x86_64-pc-windows-msvc.exe
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

# Checks that dist/ (the current directory) has every file and writes SHA256SUMS.
sums() {
    for f in "${FILES[@]}"; do
        [[ -f $f ]] || die "dist/$f is missing"
    done
    sha256sum "${FILES[@]}" >SHA256SUMS
    cat SHA256SUMS
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
    sums
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

# GitHub API call; $1 is the Accept header, the rest goes to curl.
gh_api() {
    local accept=$1
    shift
    curl -sS --header "Authorization: Bearer $GITHUB_TOKEN" --header "Accept: $accept" \
        --header "X-GitHub-Api-Version: 2022-11-28" "$@"
}

# Waits until the push mirror has brought tag $1 to GitHub, at commit $2.
gh_wait_for_tag() {
    local tag=$1 sha=$2 out status i
    out=$(mktemp)
    for ((i = 0; i < 40; i++)); do
        status=$(gh_api application/vnd.github+json -o "$out" -w '%{http_code}' \
            "$GH_REPO_API/commits/$tag")
        case $status in
            200)
                [[ $(jq -r .sha "$out") == "$sha" ]] ||
                    die "tag $tag on GitHub is at $(jq -r .sha "$out"), not $sha"
                rm -f "$out"
                return
                ;;
            404 | 422) sleep 15 ;;
            *) die "GET $GH_REPO_API/commits/$tag: HTTP $status" ;;
        esac
    done
    die "tag $tag hasn't reached GitHub after 10 minutes; check the push mirror and re-run this job"
}

# Prints the JSON of the release (drafts included) for tag $1, or nothing.
gh_find_release() {
    gh_api application/vnd.github+json --fail-with-body "$GH_REPO_API/releases?per_page=100" |
        jq --arg tag "$1" 'map(select(.tag_name == $tag)) | first // empty'
}

# Uploads file $1 to the release whose JSON is in file $2, unless the same
# file is there already.
gh_upload() {
    local file=$1 release=$2 url existing id
    url=$(jq -r --arg name "$file" '.assets[] | select(.name == $name) | .url' "$release")
    if [[ -n $url ]]; then
        existing=$(mktemp)
        gh_api application/octet-stream --fail-with-body -L -o "$existing" "$url"
        cmp -s "$file" "$existing" ||
            die "GitHub release asset $file exists with different contents; a published file is never replaced"
        rm -f "$existing"
        echo "$file: already uploaded to GitHub"
        return
    fi
    id=$(jq -r .id "$release")
    gh_api application/vnd.github+json --fail-with-body \
        -H 'Content-Type: application/octet-stream' --data-binary "@$file" \
        "https://uploads.github.com/repos/$GITHUB_REPOSITORY/releases/$id/assets?name=$file" >/dev/null
    echo "$file: uploaded to GitHub"
}

# Creates the GitHub release as a draft, uploads the files and then publishes
# it, so it never shows up without its binaries.
github() {
    local version tag notes json release
    check
    version=$(version)
    tag=v$version
    : "${GITHUB_REPOSITORY:?set GITHUB_REPOSITORY to owner/name of the GitHub mirror}"
    GH_REPO_API=https://api.github.com/repos/$GITHUB_REPOSITORY
    if [[ -z $DRY_RUN ]]; then
        [[ ${CI_COMMIT_TAG:-} == "$tag" ]] || die "github runs in a pipeline for tag $tag only; use --dry-run"
        : "${CI_COMMIT_SHA:?}" "${GITHUB_TOKEN:?set GITHUB_TOKEN (a protected, masked CI/CD variable)}"
    fi
    notes=$(mktemp)
    json=$(mktemp)
    release=$(mktemp)
    notes "$version" >"$notes"
    jq -n --arg tag "$tag" --arg name "shellsync $version" --rawfile notes "$notes" \
        '{tag_name: $tag, name: $name, body: $notes}' >"$json"

    cd dist
    sums
    if [[ -n $DRY_RUN ]]; then
        echo "would create the release on GitHub ($GITHUB_REPOSITORY) with ${FILES[*]} SHA256SUMS:"
        cat "$json"
        return
    fi

    gh_wait_for_tag "$tag" "$CI_COMMIT_SHA"
    gh_find_release "$tag" >"$release"
    if [[ -s $release ]]; then
        echo "GitHub release $tag: exists"
    else
        jq '. + {draft: true}' "$json" |
            gh_api application/vnd.github+json --fail-with-body -H 'Content-Type: application/json' \
                --data-binary @- "$GH_REPO_API/releases" >"$release"
        echo "GitHub release $tag: created as a draft"
    fi
    for f in "${FILES[@]}" SHA256SUMS; do
        gh_upload "$f" "$release"
    done
    jq '{name, body} + {draft: false}' "$json" |
        gh_api application/vnd.github+json --fail-with-body -X PATCH -H 'Content-Type: application/json' \
            --data-binary @- "$GH_REPO_API/releases/$(jq -r .id "$release")" >/dev/null
    echo "GitHub release $tag: published"
    rm -f "$notes" "$json" "$release"
}

DRY_RUN=
case ${1:-} in
    check) check ;;
    publish | github)
        case ${2:-} in
            "") ;;
            --dry-run) DRY_RUN=1 ;;
            *) die "unknown option: $2" ;;
        esac
        "$1"
        ;;
    *) die "usage: ci/release.sh check | publish [--dry-run] | github [--dry-run]" ;;
esac
