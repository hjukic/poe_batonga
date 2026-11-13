param(
    [Parameter(Mandatory = $true)][string]$ImagePath,
    [ValidateSet('full', 'selection', 'creation')][string]$Region = 'full',
    [string]$Language = 'en-US'
)

# Offline diagnostic only: reads a supplied image, never captures or controls the game.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Runtime.WindowsRuntime
Add-Type -AssemblyName System.Drawing
$null = [Windows.Storage.StorageFile, Windows.Storage, ContentType = WindowsRuntime]
$null = [Windows.Storage.Streams.IRandomAccessStreamWithContentType, Windows.Storage.Streams, ContentType = WindowsRuntime]
$null = [Windows.Graphics.Imaging.BitmapDecoder, Windows.Graphics.Imaging, ContentType = WindowsRuntime]
$null = [Windows.Graphics.Imaging.SoftwareBitmap, Windows.Graphics.Imaging, ContentType = WindowsRuntime]
$null = [Windows.Media.Ocr.OcrEngine, Windows.Foundation, ContentType = WindowsRuntime]
$null = [Windows.Media.Ocr.OcrResult, Windows.Foundation, ContentType = WindowsRuntime]
$null = [Windows.Globalization.Language, Windows.Globalization, ContentType = WindowsRuntime]

function Wait-WinRT($Operation, [type]$ResultType) {
    $converter = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
        $_.Name -eq 'AsTask' -and $_.IsGenericMethod -and
        $_.GetGenericArguments().Count -eq 1 -and $_.GetParameters().Count -eq 1 -and
        $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1'
    } | Select-Object -First 1
    $task = $converter.MakeGenericMethod($ResultType).Invoke($null, @($Operation))
    if (!$task.Wait(15000)) { throw 'Windows OCR operation timed out' }
    return $task.Result
}

$original = [IO.Path]::GetFullPath($ImagePath)
$working = $original
$temporary = $null
$offsetX = 0
$offsetY = 0
$source = [Drawing.Image]::FromFile($original)
try {
    $imageWidth = $source.Width
    $imageHeight = $source.Height
    if ($Region -ne 'full') {
        $box = if ($Region -eq 'selection') { @(0.788, 0.215, 0.202, 0.510) } else { @(0.397, 0.723, 0.210, 0.110) }
        $offsetX = [int][Math]::Floor($imageWidth * $box[0])
        $offsetY = [int][Math]::Floor($imageHeight * $box[1])
        $rectangle = [Drawing.Rectangle]::new($offsetX, $offsetY, [int]($imageWidth * $box[2]), [int]($imageHeight * $box[3]))
        $crop = ([Drawing.Bitmap]$source).Clone($rectangle, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
        try {
            $temporary = Join-Path ([IO.Path]::GetTempPath()) ('batonga-ocr-' + [Guid]::NewGuid().ToString('N') + '.png')
            $crop.Save($temporary, [Drawing.Imaging.ImageFormat]::Png)
            $working = $temporary
        } finally { $crop.Dispose() }
    }
} finally { $source.Dispose() }

$stream = $null
$bitmap = $null
try {
    $engine = [Windows.Media.Ocr.OcrEngine]::TryCreateFromLanguage([Windows.Globalization.Language]::new($Language))
    if ($null -eq $engine) { throw "Windows OCR language is not installed: $Language" }
    $file = Wait-WinRT ([Windows.Storage.StorageFile]::GetFileFromPathAsync($working)) ([Windows.Storage.StorageFile])
    $stream = Wait-WinRT ($file.OpenReadAsync()) ([Windows.Storage.Streams.IRandomAccessStreamWithContentType])
    $decoder = Wait-WinRT ([Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream)) ([Windows.Graphics.Imaging.BitmapDecoder])
    $bitmap = Wait-WinRT ($decoder.GetSoftwareBitmapAsync([Windows.Graphics.Imaging.BitmapPixelFormat]::Bgra8, [Windows.Graphics.Imaging.BitmapAlphaMode]::Premultiplied)) ([Windows.Graphics.Imaging.SoftwareBitmap])
    $result = Wait-WinRT ($engine.RecognizeAsync($bitmap)) ([Windows.Media.Ocr.OcrResult])
    $pixels = [Drawing.Bitmap]::FromFile($original)
    try {
        $lines = @($result.Lines | ForEach-Object {
            $words = @($_.Words | ForEach-Object {
                $r = $_.BoundingRect
                [pscustomobject]@{ text = $_.Text; x = $r.X + $offsetX; y = $r.Y + $offsetY; width = $r.Width; height = $r.Height }
            })
            $style = $null
            $first = $words | Select-Object -First 1
            if ($null -ne $first -and $first.x / $imageWidth -ge 0.79 -and $first.x / $imageWidth -le 0.84 -and
                $first.y / $imageHeight -ge 0.215 -and $first.y / $imageHeight -le 0.70) {
                $count = 0; $total = 0.0; $bright = 0
                for ($py = [int]$first.y; $py -lt [Math]::Min($imageHeight, $first.y + $first.height); $py++) {
                    for ($px = [int]$first.x; $px -lt [Math]::Min($imageWidth, $first.x + $first.width); $px++) {
                        $color = $pixels.GetPixel($px, $py)
                        # Gold glyphs only: a white league tooltip must not brighten a row's score.
                        if ($color.R -ge 70 -and $color.R -ge $color.G * 1.08 -and $color.G -ge $color.B * 1.15) {
                            $count++; $total += ($color.R + $color.G) / 2.0
                            if ($color.R -ge 170 -and $color.G -ge 120) { $bright++ }
                        }
                    }
                }
                $style = [pscustomobject]@{ goldPixels = $count; goldMean = $(if ($count) { [Math]::Round($total / $count, 2) } else { 0 }); brightFraction = $(if ($count) { [Math]::Round($bright / $count, 3) } else { 0 }) }
            }
            [pscustomobject]@{ text = $_.Text; words = $words; style = $style }
        })
    } finally { $pixels.Dispose() }
    [pscustomobject]@{ region = $Region; width = $imageWidth; height = $imageHeight; text = $result.Text; lines = $lines } | ConvertTo-Json -Depth 6 -Compress
} finally {
    if ($null -ne $bitmap) { $bitmap.Dispose() }
    if ($null -ne $stream) { $stream.Dispose() }
    if ($null -ne $temporary) { [IO.File]::Delete($temporary) }
}
