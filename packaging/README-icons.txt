Place your application icons here. These will be used by the
packaging scripts and cargo-bundle if present.

Expected files (dummy files are fine and can be replaced later):
- hootvoice.png   (Linux, 256x256 recommended)
- hootvoice.icns  (macOS, ICNS format)

You can generate .icns from a set of PNGs on macOS:
1) mkdir HootVoice.iconset
2) convert hootvoice.png -resize 16x16  HootVoice.iconset/icon_16x16.png
   convert hootvoice.png -resize 32x32  HootVoice.iconset/icon_16x16@2x.png
   convert hootvoice.png -resize 32x32  HootVoice.iconset/icon_32x32.png
   convert hootvoice.png -resize 64x64  HootVoice.iconset/icon_32x32@2x.png
   convert hootvoice.png -resize 128x128 HootVoice.iconset/icon_128x128.png
   convert hootvoice.png -resize 256x256 HootVoice.iconset/icon_128x128@2x.png
   convert hootvoice.png -resize 256x256 HootVoice.iconset/icon_256x256.png
   convert hootvoice.png -resize 512x512 HootVoice.iconset/icon_256x256@2x.png
   convert hootvoice.png -resize 512x512 HootVoice.iconset/icon_512x512.png
   convert hootvoice.png -resize 1024x1024 HootVoice.iconset/icon_512x512@2x.png
3) iconutil -c icns HootVoice.iconset
4) mv HootVoice.icns hootvoice.icns

