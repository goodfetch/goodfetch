// Unit tests for `readIntegrationImages` (ADR 0070; Quick suite).
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, test } from "vitest";
import { readIntegrationImages } from "./integrations.ts";

const VALID_N8N =
  '[n8n]\nrepository = "docker.io/n8nio/n8n"\ntag = "2.50.3"\ndigest = "sha256:6dc48a8dbdfe070b3babacd128995635e61cd19da9172b9aa0d5546792bd4a23"\n';
const VALID_AP =
  '[activepieces]\nrepository = "docker.io/activepieces/activepieces"\ntag = "0.94.0"\ndigest = "sha256:be0ff0426f5fa81e4c60d2ee8cef17bed2d5d053726b001949b4e5cd02c96f45"\n';

describe("readIntegrationImages", () => {
  test("reads pinned n8n and Activepieces digests from ci/integrations/images.toml", () => {
    const images = readIntegrationImages();

    expect(images.n8n.reference).toMatch(/^docker\.io\/n8nio\/n8n@sha256:[0-9a-f]{64}$/);
    expect(images.activepieces.reference).toMatch(
      /^docker\.io\/activepieces\/activepieces@sha256:[0-9a-f]{64}$/,
    );
  });

  test.for([
    {
      name: "missing digest on n8n",
      content: `${VALID_AP}\n[n8n]\nrepository = "docker.io/n8nio/n8n"\ntag = "2.50.3"\n`,
    },
    {
      name: "malformed digest on activepieces",
      content: `${VALID_N8N}\n[activepieces]\nrepository = "docker.io/activepieces/activepieces"\ntag = "0.94.0"\ndigest = "sha256:bad"\n`,
    },
    {
      name: "missing activepieces table",
      content: VALID_N8N,
    },
  ])("refuses invalid images.toml ($name)", ({ content }) => {
    const folder = mkdtempSync(join(tmpdir(), "goodfetch-pins-"));
    try {
      const badFile = join(folder, "images.toml");
      writeFileSync(badFile, content, "utf8");
      expect(() => readIntegrationImages(badFile)).toThrow(
        expect.objectContaining({ code: "integration_images_invalid" }),
      );
    } finally {
      rmSync(folder, { recursive: true, force: true });
    }
  });
});
