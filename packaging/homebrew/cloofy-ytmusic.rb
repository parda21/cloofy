# Made for each release from packaging/homebrew/cloofy-ytmusic.rb in https://github.com/parda21/cloofy.
# The YouTube Music build, without Spotify. cloofy has Spotify instead, cloofy-full both.
class CloofyYtmusic < Formula
  desc "YouTube Music client for the terminal"
  homepage "https://github.com/parda21/cloofy"
  version "@VERSION@"
  license "MIT"

  # mpv plays, yt-dlp finds the streams, and deno solves YouTube's challenges for yt-dlp.
  depends_on "deno"
  depends_on "mpv"
  depends_on "yt-dlp"

  on_macos do
    on_arm do
      url "https://github.com/parda21/cloofy/releases/download/v@VERSION@/cloofy-ytmusic-aarch64-apple-darwin.tar.gz"
      sha256 "@SHA256_YTMUSIC_AARCH64_APPLE_DARWIN@"
    end
    on_intel do
      url "https://github.com/parda21/cloofy/releases/download/v@VERSION@/cloofy-ytmusic-x86_64-apple-darwin.tar.gz"
      sha256 "@SHA256_YTMUSIC_X86_64_APPLE_DARWIN@"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/parda21/cloofy/releases/download/v@VERSION@/cloofy-ytmusic-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "@SHA256_YTMUSIC_AARCH64_UNKNOWN_LINUX_GNU@"
    end
    on_intel do
      url "https://github.com/parda21/cloofy/releases/download/v@VERSION@/cloofy-ytmusic-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "@SHA256_YTMUSIC_X86_64_UNKNOWN_LINUX_GNU@"
    end
  end

  conflicts_with "cloofy", "cloofy-full", because: "each installs a cloofy binary"

  def install
    bin.install "cloofy"
  end

  def caveats
    <<~EOS
      cloofy plays YouTube Music without an account. To also see your library
      and likes, sign in: cloofy reads your browser's YouTube cookies, and they
      stay on this computer. Then check that everything works:
        cloofy login ytmusic
        cloofy doctor
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/cloofy --version")
  end
end
