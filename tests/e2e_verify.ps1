$ErrorActionPreference = "Stop"
$scratch = "C:\Users\Acer\.gemini\antigravity\brain\becbde7a-cdc0-4269-acaf-8a38d9c044fd\scratch"
$testDir = "$scratch\e2e_all"
Remove-Item -Recurse -Force $testDir -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $testDir | Out-Null

$so = "C:\Program Files\LibreOffice\program\soffice.exe"

# 1. Create a sample .docx file
$sampleTxt = "$testDir\sample.txt"
Set-Content $sampleTxt "E2E Verification Document: Hello from Nyc-PdfWord Converter" -Encoding UTF8
& $so --headless --convert-to docx --outdir $testDir $sampleTxt | Out-Null
for ($i=0; $i -lt 30 -and -not (Test-Path "$testDir\sample.docx"); $i++) { Start-Sleep -Milliseconds 200 }

# 2. Create a sample .pdf from the docx
& $so --headless --convert-to pdf --outdir $testDir "$testDir\sample.docx" | Out-Null
for ($i=0; $i -lt 30 -and -not (Test-Path "$testDir\sample.pdf"); $i++) { Start-Sleep -Milliseconds 200 }

# 3. Create a sample .wps
Copy-Item "$testDir\sample.docx" "$testDir\sample.wps" -Force

Add-Type -AssemblyName System.IO.Compression.FileSystem

function RunTest($name, $file, $target, $fidelity, $expectedExt) {
  $args = @("-s")
  $args += @("-F", "file=@$file")
  $args += @("-F", "target=$target")
  $args += @("-F", "uid=test_suite_user")
  if ($fidelity) {
    $args += @("-F", "fidelity=$fidelity")
  }
  $args += "http://127.0.0.1:8080/api/jobs"
  
  $resp = curl.exe @args
  $id = ($resp | ConvertFrom-Json).id
  if (-not $id) {
    throw "Failed to create job for $($name): $resp"
  }
  
  $status = ""
  for ($i = 0; $i -lt 60; $i++) {
    Start-Sleep -Milliseconds 500
    $job = curl.exe -s "http://127.0.0.1:8080/api/jobs/$id" | ConvertFrom-Json
    $status = $job.status
    if ($status -in "done", "failed") { break }
  }
  
  if ($status -ne "done") {
    throw "Job failed for $($name): $($job.error)"
  }
  
  $outPath = "$testDir\result_$name.$expectedExt"
  curl.exe -s -o $outPath "http://127.0.0.1:8080/api/jobs/$id/download"
  $len = (Get-Item $outPath).Length
  if ($len -eq 0) {
    throw "Downloaded file is empty for $name"
  }
  
  return "$name -> SUCCESS ($len bytes, output: $($job.output_name))"
}

"--- Testing Module 1A: PDF -> Word (Editable) ---"
RunTest "pdf_to_word_editable" "$testDir\sample.pdf" "docx" "editable" "docx"

"--- Testing Module 1B: PDF -> Word (Exact Visual) ---"
RunTest "pdf_to_word_exact" "$testDir\sample.pdf" "docx" "exact" "docx"

"--- Testing Module 2: WPS -> PDF ---"
RunTest "wps_to_pdf" "$testDir\sample.wps" "pdf" $null "pdf"

"--- Testing Module 3: WPS -> Word ---"
RunTest "wps_to_word" "$testDir\sample.wps" "docx" $null "docx"

"--- Testing Module 4: Word -> PDF ---"
RunTest "word_to_pdf" "$testDir\sample.docx" "pdf" $null "pdf"

"--- All 4 Modules Passed Successfully! ---"
