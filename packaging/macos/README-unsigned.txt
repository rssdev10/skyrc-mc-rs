MC5000 Charger Controller - macOS Unsigned App Instructions

This package contains an unsigned macOS application bundle (.app) and disk image (.dmg).
The application has NOT been signed with an Apple Developer Certificate.

=== Installing the Application ===

1. Open MC5000Charger.dmg (or extract the app from the zip archive)
2. Drag MC5000Charger.app to your Applications folder

=== Running the Application for the First Time ===

macOS will warn about the unsigned application. To allow it to run:

METHOD 1: Right-click in Finder (Recommended)
  1. Open Finder and go to Applications
  2. Right-click on MC5000Charger.app
  3. Click "Open" (not just double-click)
  4. Click "Open" in the confirmation dialog

METHOD 2: Command Line (Terminal)
  1. Open Terminal
  2. Run: xattr -d com.apple.quarantine ~/Applications/MC5000Charger.app
  3. Then double-click the app normally

METHOD 3: System Preferences (for all warnings)
  1. Open System Preferences → Security & Privacy → General
  2. Look for "MC5000Charger.app was blocked..."
  3. Click "Allow Anyway"
  4. Double-click the app

=== Bluetooth Permissions ===

When you run the application for the first time, macOS will ask for permission to access Bluetooth.
This is required to communicate with the MC5000 battery charger. Click "OK" to allow access.

=== Running from Terminal ===

You can also run the application from the terminal:
  ~/Applications/MC5000Charger.app/Contents/MacOS/charger-controller

=== Troubleshooting ===

If the app won't open:
- Ensure you've allowed it through System Preferences
- Try the xattr command from METHOD 2 above
- Restart your Mac

If Bluetooth is not working:
- Check that Bluetooth is enabled in System Preferences
- Verify the MC5000 charger is powered on
- Check that no other application is using the Bluetooth connection

For more information, see the main README.md file.
