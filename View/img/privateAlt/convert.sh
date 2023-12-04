#!/bin/bash

while true; do
    input_file=$(find . -type f -name "*.png" | head -n 1)

    if [ -n "$input_file" ]; then
        output_file="${input_file%.*}.webp"
        ffmpeg -i "$input_file" -vf "scale=400:-1" -c:v libwebp -lossless 1 "$output_file"
        rm "$input_file"
    fi

    sleep 5
done

