#!/bin/bash
set -e

# Color output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Function to print colored output
print_info() {
	echo -e "${GREEN}[INFO]${NC} $1"
}

print_warn() {
	echo -e "${YELLOW}[WARN]${NC} $1"
}

print_error() {
	echo -e "${RED}[ERROR]${NC} $1"
}

# Function to display usage
usage() {
	cat <<EOF
Usage: $0 <version>

Bump the version in Cargo.toml, create a git tag, and push to remote.

Arguments:
  version       The new version number (e.g., 0.3.0 or v0.3.0)

Options:
  -h, --help    Show this help message
  -n, --dry-run Show what would be done without making changes

Examples:
  $0 0.3.0
  $0 v0.3.0
  $0 --dry-run 0.3.1

EOF
	exit 1
}

# Parse arguments
DRY_RUN=false
VERSION=""

while [[ $# -gt 0 ]]; do
	case $1 in
	-h | --help)
		usage
		;;
	-n | --dry-run)
		DRY_RUN=true
		shift
		;;
	*)
		if [[ -z "$VERSION" ]]; then
			VERSION="$1"
		else
			print_error "Unexpected argument: $1"
			usage
		fi
		shift
		;;
	esac
done

# Check if version was provided
if [[ -z "$VERSION" ]]; then
	print_error "Version number is required"
	usage
fi

# Strip 'v' prefix if present for Cargo.toml
VERSION_NUMBER="${VERSION#v}"

# Validate version format (semver: X.Y.Z)
if ! [[ "$VERSION_NUMBER" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
	print_error "Invalid version format: $VERSION_NUMBER"
	print_error "Expected format: X.Y.Z (e.g., 0.3.0)"
	exit 1
fi

# Git tag will always have 'v' prefix
GIT_TAG="v${VERSION_NUMBER}"

print_info "Version number: $VERSION_NUMBER"
print_info "Git tag: $GIT_TAG"

# Check if we're in a git repository
if ! git rev-parse --git-dir >/dev/null 2>&1; then
	print_error "Not in a git repository"
	exit 1
fi

# Check for uncommitted changes
if [[ -n $(git status --porcelain) ]]; then
	print_warn "You have uncommitted changes:"
	git status --short
	echo
	read -p "Continue anyway? (y/N) " -n 1 -r
	echo
	if [[ ! $REPLY =~ ^[Yy]$ ]]; then
		print_error "Aborted by user"
		exit 1
	fi
fi

# Check if tag already exists
if git rev-parse "$GIT_TAG" >/dev/null 2>&1; then
	print_error "Tag $GIT_TAG already exists"
	exit 1
fi

# Path to Cargo.toml
CARGO_TOML="rust/Cargo.toml"

if [[ ! -f "$CARGO_TOML" ]]; then
	print_error "Could not find $CARGO_TOML"
	exit 1
fi

# Get current version
CURRENT_VERSION=$(grep -m1 '^version = ' "$CARGO_TOML" | sed 's/version = "\(.*\)"/\1/')
print_info "Current version: $CURRENT_VERSION"
print_info "New version: $VERSION_NUMBER"

if [[ "$DRY_RUN" == true ]]; then
	print_warn "DRY RUN MODE - No changes will be made"
	echo
	echo "Would perform the following actions:"
	echo "  1. Update version in $CARGO_TOML: $CURRENT_VERSION -> $VERSION_NUMBER"
	echo "  2. Git add: $CARGO_TOML"
	echo "  3. Git commit: 'chore: bump version to $VERSION_NUMBER'"
	echo "  4. Git tag: $GIT_TAG"
	echo "  5. Git push: origin HEAD"
	echo "  6. Git push: origin $GIT_TAG"
	exit 0
fi

# Confirmation prompt
echo
print_warn "This will:"
echo "  1. Update version in $CARGO_TOML"
echo "  2. Commit the changes"
echo "  3. Create tag $GIT_TAG"
echo "  4. Push to remote (origin)"
echo
read -p "Proceed? (y/N) " -n 1 -r
echo
if [[ ! $REPLY =~ ^[Yy]$ ]]; then
	print_error "Aborted by user"
	exit 1
fi

# Update version in Cargo.toml
print_info "Updating version in $CARGO_TOML..."
if [[ "$OSTYPE" == "darwin"* ]]; then
	# macOS
	sed -i '' "s/^version = \".*\"/version = \"$VERSION_NUMBER\"/" "$CARGO_TOML"
else
	# Linux
	sed -i "s/^version = \".*\"/version = \"$VERSION_NUMBER\"/" "$CARGO_TOML"
fi

# Verify the change
NEW_VERSION=$(grep -m1 '^version = ' "$CARGO_TOML" | sed 's/version = "\(.*\)"/\1/')
if [[ "$NEW_VERSION" != "$VERSION_NUMBER" ]]; then
	print_error "Failed to update version in $CARGO_TOML"
	exit 1
fi

print_info "Version updated successfully"

# Git operations
print_info "Staging changes..."
git add "$CARGO_TOML"

print_info "Creating commit..."
git commit -m "chore: bump version to $VERSION_NUMBER"

print_info "Creating tag $GIT_TAG..."
git tag -a "$GIT_TAG" -m "Release $VERSION_NUMBER"

print_info "Pushing to origin..."
git push origin HEAD

print_info "Pushing tag to origin..."
git push origin "$GIT_TAG"

echo
print_info "✓ Release $VERSION_NUMBER complete!"
print_info "✓ Tag $GIT_TAG pushed to origin"
echo
print_info "GitHub Actions will now publish the crates to crates.io"
print_info "Monitor the workflow at: https://github.com/georgeharker/shareserver/actions"
