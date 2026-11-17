#!/bin/bash
cd "$(dirname "$0")"
Y="../training 1/tools/yt-dlp.exe"; F="../training 1/tools/ffmpeg-9.0.2-essentials_build/bin"
"$Y" --no-playlist -f bestaudio -x --audio-format mp3 --audio-quality 0 --ffmpeg-location "$F" -o "main max.%(ext)s" "https://youtu.be/G55cWLos73s" > mix.log 2>&1
i=0
while IFS= read -r id; do
  i=$((i+1)); n=$(printf %02d $i)
  "$Y" --no-playlist --match-filter "duration<900" --no-progress -f bestaudio -x --audio-format mp3 --audio-quality 0 --ffmpeg-location "$F" -o "tracks/$n - %(title).80s.%(ext)s" "https://www.youtube.com/watch?v=$id" >> tracks.log 2>&1
done < ids.txt
echo ALLDONE >> tracks.log
