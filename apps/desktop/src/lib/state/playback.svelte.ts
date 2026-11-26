// Playing the edit: moving the playhead in real time, with the sound in sync.
//
// The sound is the clock (decision record 0022). The engine mixes the timeline's
// sound a second at a time; Web Audio plays each chunk at an exact time, and the
// playhead is computed from the audio clock at every screen refresh. The preview
// asks for the frame under the playhead and skips frames when the computer cannot
// keep up, so pictures never drift from the sound.
//
// Playing backwards (J) has no sound: the clock is then `performance.now()`.

import { backend } from "../api/backend";
import { TICKS_PER_SECOND, frameIndexAt, ticksOfFrame } from "../time/ticks";
import { editor } from "./editor.svelte";

/** Sound is fetched this many seconds at a time. */
const SOUND_CHUNK_IN_SECONDS = 1;
/** Playback keeps at least this much sound scheduled ahead of the clock. */
const SOUND_LEAD_IN_SECONDS = 2;
/** Time given to the audio output to start the first chunk on time. */
const START_DELAY_IN_SECONDS = 0.06;
/** Shuttle speeds of J and L, pressed repeatedly. */
const SHUTTLE_SPEEDS = [1, 2, 4];

class Playback {
  isPlaying = $state(false);
  /** Positive forwards, negative backwards; 1 is normal speed. */
  speed = $state(1);
  /** True between pressing play and the first sound being ready. */
  isStarting = $state(false);

  private audioContext: AudioContext | null = null;
  /** The clock time (seconds) at which the playhead was at `startTicks`. */
  private clockStartInSeconds = 0;
  private startTicks = 0;
  private startSample = 0;
  private isClockedBySound = false;
  private nextSoundSample = 0;
  private isFetchingSound = false;
  private scheduledSources = new Set<AudioBufferSourceNode>();
  private animationFrameId = 0;
  /** Bumped on every start and stop: answers for an older run are dropped. */
  private run = 0;

  constructor() {
    editor.onViewChanged(() => {
      if (this.isPlaying) {
        void this.play(this.speed);
      }
    });
  }

  toggle(): void {
    if (this.isPlaying || this.isStarting) {
      this.pause();
    } else {
      void this.play(1);
    }
  }

  /** J (direction −1) and L (direction +1): play, then faster on each press. */
  shuttle(direction: 1 | -1): void {
    const isSameDirection = this.isPlaying && Math.sign(this.speed) === direction;
    const currentIndex = SHUTTLE_SPEEDS.indexOf(Math.abs(this.speed));
    const nextIndex = isSameDirection ? Math.min(currentIndex + 1, SHUTTLE_SPEEDS.length - 1) : 0;
    void this.play(direction * (SHUTTLE_SPEEDS[nextIndex] ?? 1));
  }

  async play(speed: number): Promise<void> {
    const view = editor.view;
    if (!view || view.durationTicks === 0) {
      return;
    }
    this.stopEverything();
    if (speed > 0 && editor.playheadTicks >= view.durationTicks) {
      editor.seek(0);
    }
    if (speed < 0 && editor.playheadTicks <= 0) {
      return;
    }
    const run = this.run;
    this.speed = speed;
    this.startTicks = editor.playheadTicks;
    this.isClockedBySound = speed > 0;
    if (this.isClockedBySound) {
      this.isStarting = true;
      const isReady = await this.prepareSound(view.settings.audioSampleRate, run);
      if (!isReady || run !== this.run) {
        return;
      }
      this.isStarting = false;
    } else {
      this.clockStartInSeconds = performance.now() / 1000;
    }
    this.isPlaying = true;
    this.animationFrameId = requestAnimationFrame(() => this.advance(run));
  }

  pause(): void {
    this.stopEverything();
    this.isPlaying = false;
    this.isStarting = false;
    this.speed = 1;
    editor.seek(editor.playheadTicks);
  }

  /** Moves the playhead by whole frames (stops playback). */
  stepFrames(frameCount: number): void {
    this.pause();
    const frameIndex = frameIndexAt(editor.playheadTicks, editor.frameRate) + frameCount;
    editor.seek(ticksOfFrame(Math.max(0, frameIndex), editor.frameRate));
  }

  /** Moves the playhead by `seconds` (stops playback). */
  stepSeconds(seconds: number): void {
    this.pause();
    editor.seek(editor.playheadTicks + seconds * TICKS_PER_SECOND);
  }

  /** Jumps to `ticks`; playback continues from there if it was playing. */
  seek(ticks: number): void {
    const wasPlaying = this.isPlaying;
    const speed = this.speed;
    this.pause();
    editor.seek(ticks);
    if (wasPlaying) {
      void this.play(speed);
    }
  }

  /** Creates the audio output, fetches the first chunk and starts the clock. */
  private async prepareSound(sampleRate: number, run: number): Promise<boolean> {
    const context = await this.audioContextAt(sampleRate);
    this.startSample = Math.floor((this.startTicks * sampleRate) / TICKS_PER_SECOND);
    this.nextSoundSample = this.startSample;
    const firstChunk = await this.fetchNextChunk(run);
    if (run !== this.run) {
      return false;
    }
    this.clockStartInSeconds = context.currentTime + START_DELAY_IN_SECONDS;
    if (firstChunk) {
      this.scheduleChunk(firstChunk.samples, firstChunk.firstSample);
    }
    return true;
  }

  private async audioContextAt(sampleRate: number): Promise<AudioContext> {
    if (this.audioContext && this.audioContext.sampleRate !== sampleRate) {
      await this.audioContext.close();
      this.audioContext = null;
    }
    if (!this.audioContext) {
      this.audioContext = new AudioContext({ sampleRate, latencyHint: "interactive" });
    }
    await this.audioContext.resume();
    return this.audioContext;
  }

  private currentClockInSeconds(): number {
    if (this.isClockedBySound && this.audioContext) {
      return this.audioContext.currentTime;
    }
    return performance.now() / 1000;
  }

  private advance(run: number): void {
    if (run !== this.run) {
      return;
    }
    const durationTicks = editor.view?.durationTicks ?? 0;
    const elapsedInSeconds = Math.max(0, this.currentClockInSeconds() - this.clockStartInSeconds);
    const position = this.startTicks + elapsedInSeconds * this.speed * TICKS_PER_SECOND;
    if (this.speed > 0 && position >= durationTicks) {
      this.pause();
      editor.seek(durationTicks);
      return;
    }
    if (this.speed < 0 && position <= 0) {
      this.pause();
      editor.seek(0);
      return;
    }
    editor.playheadTicks = position;
    if (this.isClockedBySound) {
      void this.keepSoundAhead(run);
    }
    this.animationFrameId = requestAnimationFrame(() => this.advance(run));
  }

  private async keepSoundAhead(run: number): Promise<void> {
    const context = this.audioContext;
    const view = editor.view;
    if (!context || !view || this.isFetchingSound) {
      return;
    }
    const sampleRate = context.sampleRate;
    const endSample = Math.ceil((view.durationTicks * sampleRate) / TICKS_PER_SECOND);
    const scheduledUntil = this.clockTimeOfSample(this.nextSoundSample, sampleRate);
    if (this.nextSoundSample >= endSample || scheduledUntil - context.currentTime > SOUND_LEAD_IN_SECONDS) {
      return;
    }
    const chunk = await this.fetchNextChunk(run);
    if (chunk && run === this.run) {
      this.scheduleChunk(chunk.samples, chunk.firstSample);
    }
  }

  private async fetchNextChunk(run: number): Promise<{ samples: Float32Array; firstSample: number } | null> {
    const sampleRate = this.audioContext?.sampleRate ?? 48_000;
    const firstSample = this.nextSoundSample;
    const sampleCount = Math.round(SOUND_CHUNK_IN_SECONDS * sampleRate);
    this.isFetchingSound = true;
    try {
      const samples = await backend.getTimelineSound(firstSample, sampleCount);
      if (run !== this.run) {
        return null;
      }
      this.nextSoundSample = firstSample + sampleCount;
      return { samples, firstSample };
    } catch (error) {
      if (run === this.run) {
        editor.showError(error);
        this.pause();
      }
      return null;
    } finally {
      if (run === this.run) {
        this.isFetchingSound = false;
      }
    }
  }

  /** The audio clock time at which timeline sample `sample` plays. */
  private clockTimeOfSample(sample: number, sampleRate: number): number {
    return this.clockStartInSeconds + (sample - this.startSample) / sampleRate / this.speed;
  }

  private scheduleChunk(interleavedSamples: Float32Array, firstSample: number): void {
    const context = this.audioContext;
    if (!context || interleavedSamples.length < 2) {
      return;
    }
    const frameCount = interleavedSamples.length / 2;
    const buffer = context.createBuffer(2, frameCount, context.sampleRate);
    const left = new Float32Array(frameCount);
    const right = new Float32Array(frameCount);
    for (let frame = 0; frame < frameCount; frame += 1) {
      left[frame] = interleavedSamples[frame * 2] ?? 0;
      right[frame] = interleavedSamples[frame * 2 + 1] ?? 0;
    }
    buffer.copyToChannel(left, 0);
    buffer.copyToChannel(right, 1);
    const source = context.createBufferSource();
    source.buffer = buffer;
    source.playbackRate.value = this.speed;
    source.connect(context.destination);
    // A chunk that arrives late starts part-way through, so it stays in sync.
    const startTime = this.clockTimeOfSample(firstSample, context.sampleRate);
    const lateness = Math.max(0, context.currentTime - startTime);
    source.start(Math.max(startTime, context.currentTime), lateness * this.speed);
    source.addEventListener("ended", () => this.scheduledSources.delete(source));
    this.scheduledSources.add(source);
  }

  private stopEverything(): void {
    this.run += 1;
    cancelAnimationFrame(this.animationFrameId);
    for (const source of this.scheduledSources) {
      try {
        source.stop();
      } catch {
        // Already stopped.
      }
    }
    this.scheduledSources.clear();
    this.isFetchingSound = false;
  }
}

export const playback = new Playback();
