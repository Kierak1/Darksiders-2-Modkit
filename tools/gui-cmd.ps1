param(
    [Parameter(Mandatory = $true)][string]$Command,
    [int]$Port = 47777,
    [int]$TimeoutMs = 30000
)
$client = New-Object System.Net.Sockets.TcpClient
$client.Connect("127.0.0.1", $Port)
$stream = $client.GetStream()
$stream.ReadTimeout = $TimeoutMs
$writer = New-Object System.IO.StreamWriter($stream)
$writer.AutoFlush = $true
$reader = New-Object System.IO.StreamReader($stream)
$writer.WriteLine($Command)
$sb = New-Object System.Text.StringBuilder
while ($null -ne ($line = $reader.ReadLine())) {
    if ($line -eq '.') { break }
    [void]$sb.AppendLine($line)
}
$client.Close()
Write-Output $sb.ToString().TrimEnd()
