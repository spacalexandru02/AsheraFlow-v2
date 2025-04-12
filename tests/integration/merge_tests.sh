#!/bin/bash
# Test suite for the ASH merge command
# This script tests fast-forward, recursive merge, and various conflict scenarios.

# --- Configuration ---
# Find the ASH executable
if [ -n "$1" ]; then
    ASH_EXECUTABLE="$1"
elif [ -f "./target/release/AsheraFlow" ]; then
    ASH_EXECUTABLE="$(pwd)/target/release/AsheraFlow"
elif [ -f "./target/debug/AsheraFlow" ]; then
    ASH_EXECUTABLE="$(pwd)/target/debug/AsheraFlow"
else
    echo "ASH executable not found. Build the project or provide the path as an argument."
    echo "Usage: $0 [path-to-ash-executable]"
    exit 1
fi

echo "Using ASH executable: $ASH_EXECUTABLE"
ASH_CMD="$ASH_EXECUTABLE" # Alias for easier use

set -e # Exit immediately if a command exits with a non-zero status.
# set -x # Uncomment for detailed command execution debugging

# --- Test Environment Setup ---
TEST_DIR=$(mktemp -d)
echo "Using temporary directory: ${TEST_DIR}"
cd "$TEST_DIR" || exit 1

# --- Colors and Counters ---
RED="\033[0;31m"
GREEN="\033[0;32m"
YELLOW="\033[0;33m"
BLUE="\033[0;34m"
RESET="\033[0m"
TESTS_PASSED=0
TESTS_FAILED=0

# --- Helper Functions ---
function setup_repo() {
    local repo_name=${1:-"test_repo"}
    rm -rf "$repo_name" .ash 2>/dev/null || true
    mkdir -p "$repo_name"
    cd "$repo_name"
    "$ASH_CMD" init . > /dev/null
    # Configure git user locally for commits (important for Author info)
    export GIT_AUTHOR_NAME="Test User"
    export GIT_AUTHOR_EMAIL="test@example.com"
    echo -e "${BLUE}Initialized repo in $(pwd)${RESET}"
    cd .. # Go back to TEST_DIR
}

function create_commit() {
    local repo_name="$1"
    local file_name="$2"
    local content="$3"
    local message="$4"
    local branch
    branch=$(cd "$repo_name" && cat .ash/HEAD | sed 's|ref: refs/heads/||') # Get current branch name

    echo "$content" > "$repo_name/$file_name"
    (cd "$repo_name" && "$ASH_CMD" add "$file_name" > /dev/null)
    (cd "$repo_name" && "$ASH_CMD" commit -m "$message" > /dev/null)
    echo "  Commit on '$branch': '$message' ($file_name)"
}

function run_cmd() {
    local repo_name="$1"
    shift # Remove repo_name from args
    echo -e "${YELLOW}  CMD [in $repo_name]: ${ASH_CMD} $@${RESET}"
    if (cd "$repo_name" && "$ASH_CMD" "$@") > >(tee -a stdout.log) 2> >(tee -a stderr.log >&2); then
        echo -e "${GREEN}  CMD OK${RESET}"
        return 0
    else
        local exit_code=$?
        echo -e "${RED}  CMD FAILED (Exit Code: $exit_code)${RESET}"
        cat stdout.log stderr.log # Print output on failure
        return $exit_code
    fi
}

# Function to check command success (ignores output)
function check_success() {
    local repo_name="$1"
    shift
    echo -e "${YELLOW}  CHECK SUCCESS [in $repo_name]: ${ASH_CMD} $@${RESET}"
    if (cd "$repo_name" && "$ASH_CMD" "$@") > /dev/null 2>&1; then
        echo -e "${GREEN}  CHECK OK${RESET}"
        return 0
    else
        echo -e "${RED}  CHECK FAILED${RESET}"
        return 1
    fi
}

# Function to check command failure (ignores output)
function check_failure() {
    local repo_name="$1"
    shift
    echo -e "${YELLOW}  CHECK FAILURE [in $repo_name]: ${ASH_CMD} $@${RESET}"
    if ! (cd "$repo_name" && "$ASH_CMD" "$@") > /dev/null 2>&1; then
        echo -e "${GREEN}  CHECK OK (Expected Failure)${RESET}"
        return 0
    else
        echo -e "${RED}  CHECK FAILED (Expected Failure, but Succeeded)${RESET}"
        return 1
    fi
}

function assert_file_contains() {
    local repo_name="$1"
    local file_path="$2"
    local expected_content="$3"
    local msg="$4"
    echo -e "${YELLOW}TEST: $msg${RESET}"
    if grep -qF "$expected_content" "$repo_name/$file_path"; then
        echo -e "${GREEN}PASS: $msg${RESET}"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        echo -e "${RED}FAIL: $msg - File '$repo_name/$file_path' does not contain '$expected_content'. Actual content:${RESET}"
        cat "$repo_name/$file_path"
        TESTS_FAILED=$((TESTS_FAILED + 1))
    fi
}

function assert_file_exists() {
    local repo_name="$1"
    local file_path="$2"
    local msg="$3"
     echo -e "${YELLOW}TEST: $msg${RESET}"
    if [ -f "$repo_name/$file_path" ]; then
        echo -e "${GREEN}PASS: $msg${RESET}"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        echo -e "${RED}FAIL: $msg - File '$repo_name/$file_path' does not exist.${RESET}"
        TESTS_FAILED=$((TESTS_FAILED + 1))
    fi
}

function assert_file_not_exists() {
    local repo_name="$1"
    local file_path="$2"
    local msg="$3"
     echo -e "${YELLOW}TEST: $msg${RESET}"
    if [ ! -f "$repo_name/$file_path" ] && [ ! -d "$repo_name/$file_path" ]; then
        echo -e "${GREEN}PASS: $msg${RESET}"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        echo -e "${RED}FAIL: $msg - Path '$repo_name/$file_path' exists when it shouldn't.${RESET}"
        TESTS_FAILED=$((TESTS_FAILED + 1))
    fi
}

function assert_conflict_markers() {
    local repo_name="$1"
    local file_path="$2"
    local msg="$3"
    echo -e "${YELLOW}TEST: $msg${RESET}"
    if grep -q '<<<<<<<' "$repo_name/$file_path" && grep -q '=======' "$repo_name/$file_path" && grep -q '>>>>>>>' "$repo_name/$file_path"; then
        echo -e "${GREEN}PASS: $msg${RESET}"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        echo -e "${RED}FAIL: $msg - Conflict markers not found in '$repo_name/$file_path'. Actual content:${RESET}"
        cat "$repo_name/$file_path"
        TESTS_FAILED=$((TESTS_FAILED + 1))
    fi
}

function get_head_oid() {
    local repo_name="$1"
    cat "$repo_name/.ash/refs/heads/master" 2>/dev/null || cat "$repo_name/.ash/HEAD" 2>/dev/null || echo "unknown_oid"
}

function get_branch_oid() {
    local repo_name="$1"
    local branch_name="$2"
    cat "$repo_name/.ash/refs/heads/$branch_name" 2>/dev/null || echo "unknown_oid"
}

# --- Test Cases ---

function test_fast_forward_merge() {
    echo -e "\n${BLUE}--- Test: Fast-Forward Merge ---${RESET}"
    local repo="ff_repo"
    setup_repo "$repo"
    create_commit "$repo" "file1.txt" "Initial content" "Initial commit"
    local initial_commit_oid=$(get_head_oid "$repo")

    run_cmd "$repo" branch feature
    run_cmd "$repo" checkout feature
    create_commit "$repo" "file2.txt" "Feature content" "Add feature file"
    local feature_commit_oid=$(get_branch_oid "$repo" "feature")

    run_cmd "$repo" checkout master
    run_cmd "$repo" merge feature
    local master_commit_oid=$(get_head_oid "$repo")

    assert_file_exists "$repo" "file1.txt" "FF Merge: file1.txt should exist"
    assert_file_exists "$repo" "file2.txt" "FF Merge: file2.txt should exist"
    assert_file_contains "$repo" "file2.txt" "Feature content" "FF Merge: file2.txt content check"

    if [ "$master_commit_oid" == "$feature_commit_oid" ]; then
        echo -e "${GREEN}PASS: FF Merge: master OID matches feature OID${RESET}"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        echo -e "${RED}FAIL: FF Merge: master OID ($master_commit_oid) does not match feature OID ($feature_commit_oid)${RESET}"
        TESTS_FAILED=$((TESTS_FAILED + 1))
    fi
    cd "$TEST_DIR" # Ensure we are in the base test directory
}

function test_already_up_to_date() {
    echo -e "\n${BLUE}--- Test: Already Up-to-Date Merge ---${RESET}"
    local repo="uptodate_repo"
    setup_repo "$repo"
    create_commit "$repo" "file1.txt" "Content" "Initial"
    run_cmd "$repo" branch feature
    # Merge feature into master (should be fast-forward)
    run_cmd "$repo" merge feature > /dev/null
    # Try merging again
    run_cmd "$repo" merge feature > merge_output.log
    assert_file_contains "merge_output.log" "Already up to date." "Already Up-to-Date: Output check"
    rm merge_output.log stdout.log stderr.log 2>/dev/null
    cd "$TEST_DIR"
}

function test_recursive_merge_no_conflict() {
    echo -e "\n${BLUE}--- Test: Recursive Merge (No Conflict) ---${RESET}"
    local repo="recursive_repo"
    setup_repo "$repo"
    create_commit "$repo" "common.txt" "Base" "Base commit"
    local base_oid=$(get_head_oid "$repo")

    # Master branch changes
    create_commit "$repo" "master_file.txt" "Master change" "Commit on master"
    local master_oid=$(get_head_oid "$repo")

    # Feature branch changes
    run_cmd "$repo" checkout -b feature "$base_oid" # Create feature from base
    create_commit "$repo" "feature_file.txt" "Feature change" "Commit on feature"
    local feature_oid=$(get_branch_oid "$repo" "feature")

    # Merge
    run_cmd "$repo" checkout master
    run_cmd "$repo" merge feature
    local merge_commit_oid=$(get_head_oid "$repo")

    assert_file_exists "$repo" "common.txt" "Recursive Merge: common.txt exists"
    assert_file_exists "$repo" "master_file.txt" "Recursive Merge: master_file.txt exists"
    assert_file_exists "$repo" "feature_file.txt" "Recursive Merge: feature_file.txt exists"
    assert_file_contains "$repo" "master_file.txt" "Master change" "Recursive Merge: master_file content"
    assert_file_contains "$repo" "feature_file.txt" "Feature change" "Recursive Merge: feature_file content"

    if [ "$merge_commit_oid" != "$master_oid" ] && [ "$merge_commit_oid" != "$feature_oid" ]; then
         echo -e "${GREEN}PASS: Recursive Merge: New merge commit created ($merge_commit_oid)${RESET}"
         TESTS_PASSED=$((TESTS_PASSED + 1))
         # Optionally check merge commit parents using log
    else
         echo -e "${RED}FAIL: Recursive Merge: Did not create a new merge commit.${RESET}"
         TESTS_FAILED=$((TESTS_FAILED + 1))
    fi
    cd "$TEST_DIR"
}

function test_content_conflict() {
    echo -e "\n${BLUE}--- Test: Content Conflict Merge ---${RESET}"
    local repo="conflict_repo"
    setup_repo "$repo"
    create_commit "$repo" "conflict.txt" "line1\nline2\nline3" "Base content"
    local base_oid=$(get_head_oid "$repo")

    # Master changes
    echo -e "line1_master\nline2\nline3" > "$repo/conflict.txt"
    run_cmd "$repo" add conflict.txt
    run_cmd "$repo" commit -m "Modify line 1 on master"

    # Feature changes
    run_cmd "$repo" checkout -b feature "$base_oid"
    echo -e "line1\nline2\nline3_feature" > "$repo/conflict.txt"
    run_cmd "$repo" add conflict.txt
    run_cmd "$repo" commit -m "Modify line 3 on feature"

    # Merge and expect failure
    run_cmd "$repo" checkout master
    if ! run_cmd "$repo" merge feature; then
        echo -e "${GREEN}PASS: Content Conflict: Merge command failed as expected${RESET}"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        echo -e "${RED}FAIL: Content Conflict: Merge command succeeded unexpectedly${RESET}"
        TESTS_FAILED=$((TESTS_FAILED + 1))
    fi

    assert_conflict_markers "$repo" "conflict.txt" "Content Conflict: Markers check"
    # Check index status for conflict (this requires status to show conflicts)
    # (cd "$repo" && "$ASH_CMD" status) # Manual check for now
    cd "$TEST_DIR"
}

function test_file_directory_conflict() {
    echo -e "\n${BLUE}--- Test: File/Directory Conflict Merge ---${RESET}"
    local repo="filedir_repo"
    setup_repo "$repo"
    create_commit "$repo" "dummy.txt" "dummy" "Initial"
    local base_oid=$(get_head_oid "$repo")

    # Master: Create file a/b
    mkdir "$repo/a"
    echo "master file" > "$repo/a/b"
    run_cmd "$repo" add a/b
    run_cmd "$repo" commit -m "Add file a/b on master"

    # Feature: Create file a
    run_cmd "$repo" checkout -b feature "$base_oid"
    echo "feature file" > "$repo/a"
    run_cmd "$repo" add a
    run_cmd "$repo" commit -m "Add file a on feature"

    # Merge and expect failure
    run_cmd "$repo" checkout master
    if ! run_cmd "$repo" merge feature; then
        echo -e "${GREEN}PASS: File/Dir Conflict: Merge command failed as expected${RESET}"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        echo -e "${RED}FAIL: File/Dir Conflict: Merge command succeeded unexpectedly${RESET}"
        TESTS_FAILED=$((TESTS_FAILED + 1))
    fi
    # Check for specific error message or conflicted state in index/status if implemented
    cd "$TEST_DIR"
}

function test_modify_delete_conflict() {
    echo -e "\n${BLUE}--- Test: Modify/Delete Conflict Merge ---${RESET}"
    local repo="moddel_repo"
    setup_repo "$repo"
    create_commit "$repo" "moddel.txt" "Original" "Base commit"
    local base_oid=$(get_head_oid "$repo")

    # Master: Modify file
    echo "Modified on master" > "$repo/moddel.txt"
    run_cmd "$repo" add moddel.txt
    run_cmd "$repo" commit -m "Modify on master"

    # Feature: Delete file
    run_cmd "$repo" checkout -b feature "$base_oid"
    rm "$repo/moddel.txt"
    run_cmd "$repo" add moddel.txt # Use add to record deletion
    run_cmd "$repo" commit -m "Delete on feature"

    # Merge and expect failure
    run_cmd "$repo" checkout master
    if ! run_cmd "$repo" merge feature; then
        echo -e "${GREEN}PASS: Modify/Delete Conflict: Merge command failed as expected${RESET}"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        echo -e "${RED}FAIL: Modify/Delete Conflict: Merge command succeeded unexpectedly${RESET}"
        TESTS_FAILED=$((TESTS_FAILED + 1))
    fi
    # Check for specific error message or conflicted state in index/status if implemented
    cd "$TEST_DIR"
}

function test_merge_fail_untracked_overwrite() {
    echo -e "\n${BLUE}--- Test: Merge Fail (Untracked Overwrite) ---${RESET}"
    local repo="untracked_repo"
    setup_repo "$repo"
    create_commit "$repo" "common.txt" "Base" "Base commit"
    local base_oid=$(get_head_oid "$repo")
    create_commit "$repo" "master_file.txt" "Master" "Master change"
    run_cmd "$repo" checkout -b feature "$base_oid"
    create_commit "$repo" "feature_file.txt" "Feature" "Feature change" # This file will conflict

    # Create untracked file that merge would create
    run_cmd "$repo" checkout master
    echo "Untracked content" > "$repo/feature_file.txt"

    # Merge and expect failure
    if ! run_cmd "$repo" merge feature; then
        echo -e "${GREEN}PASS: Untracked Overwrite: Merge command failed as expected${RESET}"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        echo -e "${RED}FAIL: Untracked Overwrite: Merge command succeeded unexpectedly${RESET}"
        TESTS_FAILED=$((TESTS_FAILED + 1))
    fi
    assert_file_contains "$repo" "feature_file.txt" "Untracked content" "Untracked Overwrite: File should remain unchanged"
    cd "$TEST_DIR"
}

function test_merge_fail_uncommitted_changes() {
    echo -e "\n${BLUE}--- Test: Merge Fail (Uncommitted Changes) ---${RESET}"
    local repo="uncommitted_repo"
    setup_repo "$repo"
    create_commit "$repo" "common.txt" "Base" "Base commit"
    local base_oid=$(get_head_oid "$repo")
    create_commit "$repo" "master_file.txt" "Master" "Master change"
    run_cmd "$repo" checkout -b feature "$base_oid"
    create_commit "$repo" "feature_file.txt" "Feature" "Feature change"

    # Modify tracked file without committing
    run_cmd "$repo" checkout master
    echo "Uncommitted modification" >> "$repo/master_file.txt"

    # Merge and expect failure
    if ! run_cmd "$repo" merge feature; then
        echo -e "${GREEN}PASS: Uncommitted Changes: Merge command failed as expected${RESET}"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        echo -e "${RED}FAIL: Uncommitted Changes: Merge command succeeded unexpectedly${RESET}"
        TESTS_FAILED=$((TESTS_FAILED + 1))
    fi
    assert_file_contains "$repo" "master_file.txt" "Uncommitted modification" "Uncommitted Changes: File should retain changes"
    cd "$TEST_DIR"
}


# --- Run Tests ---
# Clear logs for new run
rm -f stdout.log stderr.log

test_fast_forward_merge
test_already_up_to_date
test_recursive_merge_no_conflict
test_content_conflict
test_file_directory_conflict
test_modify_delete_conflict
test_merge_fail_untracked_overwrite
test_merge_fail_uncommitted_changes

# --- Summary ---
echo -e "\n${BLUE}--- Test Summary ---${RESET}"
echo -e "${GREEN}Tests Passed: $TESTS_PASSED${RESET}"
if [ "$TESTS_FAILED" -gt 0 ]; then
    echo -e "${RED}Tests Failed: $TESTS_FAILED${RESET}"
else
    echo -e "${GREEN}Tests Failed: $TESTS_FAILED${RESET}"
fi

# --- Cleanup ---
cd ..
rm -rf "$TEST_DIR"
echo "Cleaned up temporary directory: $TEST_DIR"

# Exit with status code indicating failure if any tests failed
if [ "$TESTS_FAILED" -gt 0 ]; then
    exit 1
else
    exit 0
fi