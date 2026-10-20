import { readFeatureIds } from "@goodfetch/test-support/features";

export default {
  test: {
    include: ["web/**/*.test.ts"],
    tags: readFeatureIds(import.meta.dirname).map((name) => ({ name })),
    strictTags: true,
  },
};
