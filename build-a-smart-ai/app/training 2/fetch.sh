#!/bin/bash
cd "$(dirname "$0")"
Y="../training 1/tools/yt-dlp.exe"; F="../training 1/tools/ffmpeg-9.0.2-essentials_build/bin"
"$Y" --no-playlist -f bestaudio -x --audio-format mp3 --audio-quality 0 --ffmpeg-location "$F" -o "main mix.%(ext)s" "https://youtu.be/ETsmAnCfY68" > mix.log 2>&1
i=0
while IFS= read -r q; do
  i=$((i+1)); n=$(printf %02d $i)
  "$Y" "ytsearch6:$q" --match-filter "duration>60 & duration<600" --max-downloads 1 --no-progress -f bestaudio -x --audio-format mp3 --audio-quality 0 --ffmpeg-location "$F" -o "tracks/$n - %(title).80s.%(ext)s" >> tracks.log 2>&1
  echo "$n $q" >> tracks.log
done < tracklist.txt
echo ALLDONE >> tracks.log
