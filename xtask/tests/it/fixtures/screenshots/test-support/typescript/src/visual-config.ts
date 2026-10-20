// A fixture workspace's shared visual config, the one file where the
// screenshot comparisons' tolerances are set (ADR 0073).
export const playwrightExpect = {
  toHaveScreenshot: { maxDiffPixels: 0, maxDiffPixelRatio: 0, threshold: 0.2 },
};

export const vitestExpect = {
  toMatchScreenshot: {
    comparatorName: "pixelmatch",
    comparatorOptions: {
      allowedMismatchedPixels: 0,
      allowedMismatchedPixelRatio: 0,
      threshold: 0.1,
    },
  },
};
