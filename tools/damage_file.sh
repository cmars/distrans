#!/usr/bin/env bash

# Damage random blocks in a file to test index diff & reconciliation.

# Function to corrupt a file
corrupt_file() {
    local file_path=$1
    local block_size=${2:-1024}  # Default block size is 1024 bytes
    local num_blocks=${3:-5}     # Default number of blocks to corrupt is 5

    local file_size=$(stat -c%s "$file_path")
    if (( file_size < block_size * num_blocks )); then
        echo "File size is too small to corrupt the specified number of blocks."
        exit 1
    fi

    for (( i=0; i<num_blocks; i++ )); do
        local random_offset=$((RANDOM % (file_size - block_size)))
        dd if=/dev/urandom of="$file_path" bs="$block_size" count=1 seek="$random_offset" conv=notrunc 2>/dev/null
    done
}

# Check if the correct number of arguments is provided
if [ "$#" -lt 1 ]; then
    echo "Usage: $0 <file_path> [block_size] [num_blocks]"
    exit 1
fi

# Call the function with the provided arguments
corrupt_file "$@"