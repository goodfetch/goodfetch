// The codec page's script (ADR 0074): for each sample `/samples.json`
// lists, it asks `canPlayType` and Media Capabilities about the sample's
// type, decodes the sample, and compares what it decoded with the expected
// result. A video's first frame is drawn to a canvas and read at its
// center; an audio sample is decoded with Web Audio and its frequency read
// from its upward zero crossings. It shows each sample's answers, observed
// result and whether it `decoded` to the expected one as JSON in
// `#results`, by name, and sets `data-codecs="done"` on the root element.
"use strict";

/** How long a video may take to show its first frame. */
const LOAD_MS = 5000;

/** The sample rate audio is decoded at. */
const SAMPLE_RATE = 48_000;

/** Media Capabilities' answer for a sample, or the error it threw. */
const capabilities = async (sample) => {
  const configuration =
    sample.kind === "video"
      ? {
          type: "file",
          video: {
            contentType: sample.type,
            width: sample.expected.width,
            height: sample.expected.height,
            bitrate: 100_000,
            framerate: 10,
          },
        }
      : {
          type: "file",
          audio: {
            contentType: sample.type,
            channels: String(sample.expected.channels),
            bitrate: 64_000,
            samplerate: SAMPLE_RATE,
          },
        };
  try {
    const { supported, smooth, powerEfficient } =
      await navigator.mediaCapabilities.decodingInfo(configuration);
    return { supported, smooth, powerEfficient };
  } catch (error) {
    return { error: String(error) };
  }
};

/** Resolves once `video` shows its first frame; rejects on an error or after `LOAD_MS`. */
const firstFrame = (video) =>
  new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("no frame in time")), LOAD_MS);
    video.addEventListener("loadeddata", () => {
      clearTimeout(timer);
      resolve();
    });
    video.addEventListener("error", () => {
      clearTimeout(timer);
      reject(new Error(`media error ${video.error?.code}`));
    });
  });

/** A video sample's size, duration and first frame's color at its center. */
const decodeVideo = async (url) => {
  const video = document.createElement("video");
  video.muted = true;
  video.preload = "auto";
  const loaded = firstFrame(video);
  video.src = url;
  await loaded;
  const canvas = document.createElement("canvas");
  canvas.width = video.videoWidth;
  canvas.height = video.videoHeight;
  const context = canvas.getContext("2d", { willReadFrequently: true });
  context.drawImage(video, 0, 0);
  const [red, green, blue] = context.getImageData(
    Math.floor(canvas.width / 2),
    Math.floor(canvas.height / 2),
    1,
    1,
  ).data;
  return {
    width: video.videoWidth,
    height: video.videoHeight,
    duration: video.duration,
    color: [red, green, blue],
  };
};

/** An audio sample's channels, duration and frequency from its upward zero crossings. */
const decodeAudio = async (url) => {
  const bytes = await (await fetch(url)).arrayBuffer();
  const context = new OfflineAudioContext(1, SAMPLE_RATE, SAMPLE_RATE);
  const buffer = await context.decodeAudioData(bytes);
  const data = buffer.getChannelData(0);
  const crossings = [];
  for (let index = 1; index < data.length; index += 1) {
    if (data[index - 1] < 0 && data[index] >= 0) {
      crossings.push(index);
    }
  }
  // The median period, so the ringing a lossy encoder leaves in its padding
  // does not count.
  const periods = crossings.slice(1).map((crossing, index) => crossing - crossings[index]);
  periods.sort((left, right) => left - right);
  const period = periods[Math.floor(periods.length / 2)];
  return {
    channels: buffer.numberOfChannels,
    duration: buffer.duration,
    frequency: period === undefined ? 0 : buffer.sampleRate / period,
  };
};

/** Whether `observed` is the `expected` result, within `tolerance`. */
const matches = (expected, observed, tolerance) =>
  Object.entries(expected).every(([field, value]) => {
    const seen = observed[field];
    if (field === "color") {
      return value.every((channel, index) => Math.abs(channel - seen[index]) <= tolerance.color);
    }
    if (field in tolerance) {
      return Math.abs(value - seen) <= tolerance[field];
    }
    return value === seen;
  });

const run = async () => {
  const { tolerance, samples } = await (await fetch("/samples.json")).json();
  const results = {};
  for (const sample of samples) {
    const element = document.createElement(sample.kind);
    const url = `/samples/${sample.file}`;
    let observed = {};
    try {
      observed = await (sample.kind === "video" ? decodeVideo(url) : decodeAudio(url));
    } catch (error) {
      observed = { error: String(error) };
    }
    results[sample.name] = {
      can_play_type: element.canPlayType(sample.type),
      media_capabilities: await capabilities(sample),
      expected: sample.expected,
      observed,
      decoded: !("error" in observed) && matches(sample.expected, observed, tolerance),
    };
  }
  document.getElementById("results").textContent = JSON.stringify(results);
  document.documentElement.dataset.codecs = "done";
};

void run();
