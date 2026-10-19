cask "ai-toolbox" do
  version "1.2.0"

  on_arm do
    sha256 "d1fad9830a32b4708cc75bec9d356ce09300a4d4482e28cc19fc3a32864e7806"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_1.2.0_aarch64.dmg"
  end

  on_intel do
    sha256 "4d65b157406d4ff9fae7d2b0671fc3ccdec619cc9b398ff4c7be0f4686e4c449"
    url "https://github.com/coulsontl/ai-toolbox/releases/download/v#{version}/AI.Toolbox_1.2.0_x64.dmg"
  end

  name "AI Toolbox"
  desc "Desktop toolbox for managing AI coding assistant configurations"
  homepage "https://github.com/coulsontl/ai-toolbox"

  app "AI Toolbox.app"
end
