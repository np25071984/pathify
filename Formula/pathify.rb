class Pathify < Formula
  desc "Local-first CLI and TUI for GPS trace data"
  homepage "https://github.com/np25071984/pathify"
  url "https://github.com/np25071984/pathify/archive/refs/tags/v1.4.0.tar.gz"
  sha256 "4e1960316f3751a506f92cba4eedef3c92e9da713d4739e9e786964b8952c071"
  license "MIT"
  head "https://github.com/np25071984/pathify.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/pathify-cli")
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/pathify --version")

    gpx = testpath/"ride.gpx"
    gpx.write <<~XML
      <?xml version="1.0"?>
      <gpx version="1.1" creator="test" xmlns="http://www.topografix.com/GPX/1/1">
        <trk><trkseg>
          <trkpt lat="47.6062" lon="-122.3321"><ele>56.0</ele></trkpt>
          <trkpt lat="47.6070" lon="-122.3330"><ele>61.0</ele></trkpt>
        </trkseg></trk>
      </gpx>
    XML
    assert_match(/points\s+2/, shell_output("#{bin}/pathify info #{gpx}"))
  end
end
