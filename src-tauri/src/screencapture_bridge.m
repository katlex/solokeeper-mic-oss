/// ScreenCaptureKit bridge for capturing system audio on macOS 13+.
///
/// Uses SCStream with audio-only output to capture all system audio
/// (excluding the current process). Writes mono 16-bit PCM WAV directly.
/// Exposes a C API consumed by screencapture.rs via FFI.

#import <ScreenCaptureKit/ScreenCaptureKit.h>
#import <CoreMedia/CoreMedia.h>
#import <AudioToolbox/AudioToolbox.h>
#include <stdio.h>
#include <string.h>

// ---------------------------------------------------------------------------
// WAV file helpers
// ---------------------------------------------------------------------------

/// Write a 44-byte WAV header. Called once at open (placeholder sizes)
/// and again at finalize (correct sizes).
static void wav_write_header(FILE *f, uint32_t sample_rate, uint32_t data_size) {
    uint8_t h[44];
    memset(h, 0, 44);

    memcpy(h,      "RIFF", 4);
    uint32_t file_size = 36 + data_size;
    memcpy(h + 4,  &file_size, 4);
    memcpy(h + 8,  "WAVE", 4);

    memcpy(h + 12, "fmt ", 4);
    uint32_t fmt_size = 16;
    memcpy(h + 16, &fmt_size, 4);
    uint16_t pcm = 1;
    memcpy(h + 20, &pcm, 2);            // format = PCM
    uint16_t ch = 1;
    memcpy(h + 22, &ch, 2);             // mono
    memcpy(h + 24, &sample_rate, 4);
    uint32_t byte_rate = sample_rate * 2;
    memcpy(h + 28, &byte_rate, 4);      // byte rate (16-bit mono)
    uint16_t block_align = 2;
    memcpy(h + 32, &block_align, 2);
    uint16_t bits = 16;
    memcpy(h + 34, &bits, 2);

    memcpy(h + 36, "data", 4);
    memcpy(h + 40, &data_size, 4);

    fseek(f, 0, SEEK_SET);
    fwrite(h, 1, 44, f);
}

// ---------------------------------------------------------------------------
// SCStreamOutput / SCStreamDelegate handler
// ---------------------------------------------------------------------------

API_AVAILABLE(macos(13.0))
@interface SystemAudioCapture : NSObject <SCStreamOutput, SCStreamDelegate>
@property (nonatomic, strong) SCStream *stream;
@property (nonatomic) FILE       *wavFile;
@property (nonatomic) uint32_t    dataSize;
@property (nonatomic) uint32_t    sampleRate;
@property (nonatomic) uint32_t    actualSampleRate;
@property (nonatomic) BOOL        headerFixedUp;
@property (nonatomic) float       currentLevel;
@property (atomic)    BOOL        capturing;
@end

API_AVAILABLE(macos(13.0))
@implementation SystemAudioCapture

- (void)stream:(SCStream *)stream
    didOutputSampleBuffer:(CMSampleBufferRef)sampleBuffer
                   ofType:(SCStreamOutputType)type
{
    if (type != SCStreamOutputTypeAudio || !self.capturing || !self.wavFile)
        return;

    CMBlockBufferRef blockBuf = CMSampleBufferGetDataBuffer(sampleBuffer);
    if (!blockBuf) return;

    size_t totalLen = 0;
    char  *rawPtr   = NULL;
    if (CMBlockBufferGetDataPointer(blockBuf, 0, NULL, &totalLen, &rawPtr) != noErr)
        return;

    CMFormatDescriptionRef fmtDesc = CMSampleBufferGetFormatDescription(sampleBuffer);
    const AudioStreamBasicDescription *asbd =
        CMAudioFormatDescriptionGetStreamBasicDescription(fmtDesc);
    if (!asbd) return;

    // ScreenCaptureKit delivers Float32 LPCM (may be interleaved or non-interleaved)
    if (asbd->mFormatID != kAudioFormatLinearPCM)      return;
    if (!(asbd->mFormatFlags & kAudioFormatFlagIsFloat)) return;

    uint32_t channels       = asbd->mChannelsPerFrame;
    uint32_t incomingRate   = (uint32_t)asbd->mSampleRate;
    BOOL     isNonInterleaved = (asbd->mFormatFlags & kAudioFormatFlagIsNonInterleaved) != 0;

    // Log format details on first callback
    if (!self.headerFixedUp) {
        NSLog(@"[screencapture] Audio format: %.0f Hz, %u ch, flags=0x%x, bytesPerFrame=%u, bytesPerPacket=%u, %s",
              asbd->mSampleRate, channels, (unsigned)asbd->mFormatFlags,
              (unsigned)asbd->mBytesPerFrame, (unsigned)asbd->mBytesPerPacket,
              isNonInterleaved ? "NON-INTERLEAVED" : "INTERLEAVED");
        self.actualSampleRate = incomingRate;
        self.headerFixedUp    = YES;
        if (incomingRate != self.sampleRate) {
            NSLog(@"[screencapture] Actual sample rate: %u (requested %u), fixing WAV header",
                  incomingRate, self.sampleRate);
            self.sampleRate = incomingRate;
            wav_write_header(self.wavFile, incomingRate, 0);
        }
    }

    float   *floats     = (float *)rawPtr;
    float    maxLevel   = 0.0f;

    if (isNonInterleaved) {
        // Non-interleaved: each channel is a separate plane in the block buffer.
        // CMBlockBuffer may still concatenate them: [ch0_frame0..ch0_frameN][ch1_frame0..ch1_frameN]
        // But for SCStream audio, it's typically delivered via AudioBufferList.
        // When accessed via CMBlockBufferGetDataPointer, non-interleaved data with
        // bytesPerFrame == sizeof(float) means each "frame" is 1 float (one channel).
        // The total frames across all channels = totalLen / sizeof(float).
        // Frames per channel = totalLen / sizeof(float) / channels.
        size_t framesPerChannel = totalLen / (sizeof(float) * channels);

        for (size_t i = 0; i < framesPerChannel; i++) {
            float mono = 0.0f;
            for (uint32_t ch = 0; ch < channels; ch++) {
                mono += floats[ch * framesPerChannel + i];
            }
            mono /= (float)channels;

            float absVal = fabsf(mono);
            if (absVal > maxLevel) maxLevel = absVal;

            int32_t clamped = (int32_t)(mono * 32767.0f);
            if (clamped >  32767) clamped =  32767;
            if (clamped < -32768) clamped = -32768;
            int16_t s16 = (int16_t)clamped;

            fwrite(&s16, sizeof(int16_t), 1, self.wavFile);
            self.dataSize += sizeof(int16_t);
        }
    } else {
        // Interleaved: [L0,R0,L1,R1,...]
        size_t frameCount = totalLen / (sizeof(float) * channels);

        for (size_t i = 0; i < frameCount; i++) {
            float mono = 0.0f;
            for (uint32_t ch = 0; ch < channels; ch++) {
                mono += floats[i * channels + ch];
            }
            mono /= (float)channels;

            float absVal = fabsf(mono);
            if (absVal > maxLevel) maxLevel = absVal;

            int32_t clamped = (int32_t)(mono * 32767.0f);
            if (clamped >  32767) clamped =  32767;
            if (clamped < -32768) clamped = -32768;
            int16_t s16 = (int16_t)clamped;

            fwrite(&s16, sizeof(int16_t), 1, self.wavFile);
            self.dataSize += sizeof(int16_t);
        }
    }

    self.currentLevel = maxLevel;
}

- (void)stream:(SCStream *)stream didStopWithError:(NSError *)error {
    NSLog(@"[screencapture] Stream stopped with error: %@", error);
    self.capturing = NO;
}

/// Open WAV file and write initial header.
- (BOOL)openWavFile:(const char *)path sampleRate:(uint32_t)rate {
    self.wavFile    = fopen(path, "wb");
    if (!self.wavFile) return NO;
    self.dataSize   = 0;
    self.sampleRate = rate;
    wav_write_header(self.wavFile, rate, 0); // placeholder
    return YES;
}

/// Seek back and rewrite the WAV header with correct sizes, then close.
- (void)finalizeWav {
    if (!self.wavFile) return;
    wav_write_header(self.wavFile, self.sampleRate, self.dataSize);
    fclose(self.wavFile);
    self.wavFile = NULL;
    NSLog(@"[screencapture] WAV finalized: %u bytes of audio data", self.dataSize);
}

@end

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

static SystemAudioCapture *g_capture API_AVAILABLE(macos(13.0)) = nil;

// ---------------------------------------------------------------------------
// C API (called from Rust via FFI)
// ---------------------------------------------------------------------------

/// Start capturing system audio, writing mono 16-bit PCM to `output_path`.
/// Blocks until capture is running (up to 10 s timeout).
/// Returns 0 on success, -1 on error, -2 if macOS < 13.
int scb_start_capture(const char *output_path) {
    if (@available(macOS 13.0, *)) {
        if (g_capture && g_capture.capturing) {
            NSLog(@"[screencapture] Already capturing");
            return -1;
        }

        __block int result = -1;
        dispatch_semaphore_t sem = dispatch_semaphore_create(0);

        const uint32_t sampleRate = 48000;

        [SCShareableContent getShareableContentWithCompletionHandler:
            ^(SCShareableContent * _Nullable content, NSError * _Nullable error)
        {
            if (error || content.displays.count == 0) {
                NSLog(@"[screencapture] Failed to get shareable content: %@", error);
                dispatch_semaphore_signal(sem);
                return;
            }

            SCDisplay *display = content.displays.firstObject;

            SCContentFilter *filter =
                [[SCContentFilter alloc] initWithDisplay:display
                                excludingApplications:@[]
                                    exceptingWindows:@[]];

            SCStreamConfiguration *config = [[SCStreamConfiguration alloc] init];
            config.capturesAudio              = YES;
            config.excludesCurrentProcessAudio = YES;
            config.channelCount               = 2;   // stereo in, we mix to mono
            config.sampleRate                 = sampleRate;
            // Minimise video overhead (can't disable video entirely)
            config.width  = 2;
            config.height = 2;
            config.minimumFrameInterval = CMTimeMake(1, 1); // 1 fps
            config.showsCursor = NO;

            SystemAudioCapture *capture = [[SystemAudioCapture alloc] init];
            capture.headerFixedUp    = NO;
            capture.actualSampleRate = sampleRate;
            if (![capture openWavFile:output_path sampleRate:sampleRate]) {
                NSLog(@"[screencapture] Failed to open WAV file: %s", output_path);
                dispatch_semaphore_signal(sem);
                return;
            }

            SCStream *stream =
                [[SCStream alloc] initWithFilter:filter
                                   configuration:config
                                        delegate:capture];

            dispatch_queue_t queue =
                dispatch_queue_create("com.solokeeper.screencapture",
                                     DISPATCH_QUEUE_SERIAL);

            NSError *addErr = nil;
            [stream addStreamOutput:capture
                               type:SCStreamOutputTypeAudio
                  sampleHandlerQueue:queue
                               error:&addErr];
            if (addErr) {
                NSLog(@"[screencapture] addStreamOutput error: %@", addErr);
                [capture finalizeWav];
                dispatch_semaphore_signal(sem);
                return;
            }

            capture.stream    = stream;
            capture.capturing = YES;
            g_capture         = capture;

            [stream startCaptureWithCompletionHandler:^(NSError * _Nullable startErr) {
                if (startErr) {
                    NSLog(@"[screencapture] startCapture error: %@", startErr);
                    capture.capturing = NO;
                    [capture finalizeWav];
                    g_capture = nil;
                } else {
                    NSLog(@"[screencapture] Capture started at %u Hz", sampleRate);
                    result = 0;
                }
                dispatch_semaphore_signal(sem);
            }];
        }];

        // Wait up to 10 seconds for the async chain to complete
        dispatch_semaphore_wait(sem,
            dispatch_time(DISPATCH_TIME_NOW, 10 * NSEC_PER_SEC));
        return result;

    } else {
        NSLog(@"[screencapture] ScreenCaptureKit requires macOS 13.0+");
        return -2;
    }
}

/// Stop capture and finalize WAV. Blocks until done (up to 5 s).
void scb_stop_capture(void) {
    if (@available(macOS 13.0, *)) {
        if (!g_capture || !g_capture.capturing) return;

        g_capture.capturing = NO;

        dispatch_semaphore_t sem = dispatch_semaphore_create(0);

        [g_capture.stream stopCaptureWithCompletionHandler:^(NSError * _Nullable error) {
            if (error) {
                NSLog(@"[screencapture] stopCapture error: %@", error);
            }
            [g_capture finalizeWav];
            NSLog(@"[screencapture] Capture stopped");
            dispatch_semaphore_signal(sem);
        }];

        dispatch_semaphore_wait(sem,
            dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC));
        g_capture = nil;
    }
}

/// Returns 1 if currently capturing, 0 otherwise.
int scb_is_capturing(void) {
    if (@available(macOS 13.0, *)) {
        return (g_capture && g_capture.capturing) ? 1 : 0;
    }
    return 0;
}

/// Returns the current peak audio level (0.0 – 1.0).
float scb_get_level(void) {
    if (@available(macOS 13.0, *)) {
        return g_capture ? g_capture.currentLevel : 0.0f;
    }
    return 0.0f;
}
