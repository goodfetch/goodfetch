// Public entry point for Activepieces piece `goodfetch-collie` (ADRs 0027,
// 0070, 0090, 0095; spec #608, issue 1409). Built with the official
// `@activepieces/pieces-framework` `createPiece` factory.
import { createPiece, Piece } from "@activepieces/pieces-framework";
import { getRunProgressAction } from "./actions/get-run-progress.ts";
import { getRunReportAction } from "./actions/get-run-report.ts";
import { startCatalogRunAction } from "./actions/start-catalog-run.ts";
import { startRecipeAction } from "./actions/start-recipe.ts";
import { goodfetchCollieAuth } from "./common/client.ts";
import { notificationTrigger } from "./triggers/notification.ts";

export const PIECE_NAME = "goodfetch-collie" as const;

export const goodfetchColliePiece: Piece<typeof goodfetchCollieAuth> = createPiece({
  displayName: "Goodfetch Collie",
  description:
    "Start Collie browser automation runs, read reports/progress, and receive Notifications.",
  logoUrl: "https://docs.goodfetch.dev/img/collie-logo.svg",
  authors: ["Goodfetch"],
  auth: goodfetchCollieAuth,
  actions: [startRecipeAction, startCatalogRunAction, getRunReportAction, getRunProgressAction],
  triggers: [notificationTrigger],
});

export {
  getRunProgressAction,
  getRunReportAction,
  startCatalogRunAction,
  startRecipeAction,
  notificationTrigger,
  goodfetchCollieAuth,
};
export {
  ActivepiecesCollieError,
  createCollieClient,
  extractCollieAuth,
  mapCatalogErrorToActivepieces,
  toActivepiecesError,
  type ActivepiecesCollieAuth,
} from "./common/client.ts";
export type { GetRunProgressProps } from "./actions/get-run-progress.ts";
export type { GetRunReportProps } from "./actions/get-run-report.ts";
export type { StartCatalogRunProps } from "./actions/start-catalog-run.ts";
export type { StartRecipeProps } from "./actions/start-recipe.ts";
export type { NotificationTriggerProps } from "./triggers/notification.ts";
