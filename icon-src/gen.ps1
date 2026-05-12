Add-Type -AssemblyName System.Drawing
$size = 1024
$bmp = New-Object System.Drawing.Bitmap($size, $size)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
$g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic

# Dark grey background, rounded square
$bgColor = [System.Drawing.Color]::FromArgb(255, 26, 26, 26)
$bgBrush = New-Object System.Drawing.SolidBrush($bgColor)
$radius = 180
$path = New-Object System.Drawing.Drawing2D.GraphicsPath
$d = $radius * 2
$path.AddArc(0, 0, $d, $d, 180, 90)
$path.AddArc($size - $d, 0, $d, $d, 270, 90)
$path.AddArc($size - $d, $size - $d, $d, $d, 0, 90)
$path.AddArc(0, $size - $d, $d, $d, 90, 90)
$path.CloseFigure()
$g.FillPath($bgBrush, $path)

# Amber square dot, offset upper-left quadrant
$dotColor = [System.Drawing.Color]::FromArgb(255, 192, 138, 62)
$dotBrush = New-Object System.Drawing.SolidBrush($dotColor)
$dotSize = 220
$dotX = 240
$dotY = 240
$g.FillRectangle($dotBrush, $dotX, $dotY, $dotSize, $dotSize)

$bmp.Save("$PWD\icon-src\icon.png", [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose()
$bmp.Dispose()
"Wrote $PWD\icon-src\icon.png"
