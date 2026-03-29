# macOS Audio Setup

To record both your voice AND remote participants, create an **Aggregate Device** in Audio MIDI Setup.

## Step 1: Create Aggregate Device

1. Open **Audio MIDI Setup** (Spotlight → "Audio MIDI Setup")
2. Click **+** at bottom left → **Create Aggregate Device**
3. Check both:
   - ✅ Your microphone (e.g., "MacBook Pro Microphone" or "Beats Flex")
   - ✅ BlackHole 2ch
4. Name it something like **"SoloKeeper Input"**
5. Enable **Drift Correction** on BlackHole 2ch

## Step 2: Keep Multi-Output Device for Speakers

You should already have a Multi-Output Device (so you hear audio AND it goes to BlackHole):
- ✅ Your speakers/headphones (first!)
- ✅ BlackHole 2ch

Set this as your **System Output** in Sound Settings.

## Step 3: Configure SoloKeeper Mic

In Settings:
- **Microphone:** Select **"SoloKeeper Input"** (the Aggregate Device)
- **System Audio Device:** Leave empty / None

The Aggregate Device captures both mic + BlackHole in one stream — no conflicts!

## Why?

Two separate cpal audio streams on macOS can cause buffer underruns (choppy audio).
Using one Aggregate Device stream is the reliable solution.
