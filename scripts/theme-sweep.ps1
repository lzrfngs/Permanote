$path = 'C:\dev\permanote\src\routes\+page.svelte'
$lines = Get-Content -LiteralPath $path
$skipPrefixes = @('--bg:','--fg:','--fg-dim:','--fg-faint:','--panel:','--panel-2:','--input-bg:','--hover-bg:','--accent-bg:','--border:','--border-2:','--border-3:')
$pairs = @(
  @('#0a0a0a','var(--panel)'),
  @('#050505','var(--input-bg)'),
  @('#131313','var(--panel-2)'),
  @('#161616','var(--hover-bg)'),
  @('#181818','var(--panel-2)'),
  @('#1a1a1a','var(--border)'),
  @('#1d1d1d','var(--accent-bg)'),
  @('#1f1f1f','var(--hover-bg)'),
  @('#2a2a2a','var(--border-2)'),
  @('#3a3a3a','var(--border-3)'),
  @('#222','var(--border)'),
  @('#333','var(--border-2)'),
  @('#444','var(--border-3)')
)
$out = foreach ($line in $lines) {
  $trim = $line.TrimStart()
  $skip = $false
  foreach ($p in $skipPrefixes) { if ($trim.StartsWith($p)) { $skip = $true; break } }
  if (-not $skip) {
    foreach ($pair in $pairs) {
      $pattern = [regex]::Escape($pair[0]) + '\b'
      $line = $line -replace $pattern, $pair[1]
    }
  }
  $line
}
Set-Content -LiteralPath $path -Value $out -Encoding UTF8
'done'
