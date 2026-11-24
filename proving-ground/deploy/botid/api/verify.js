// `POST /botid/verify` (#168): asks Vercel BotID about the request, then
// forwards its verdict to the online copy's `/botid/verify` in
// `x-proving-ground-botid` (`human`, `bot`, or `bypassed` when BotID did not
// judge it, such as outside production), beside the edge's secret in
// `x-proving-ground-edge`, and answers with the copy's verdict page. The copy
// judges, never this function: it trusts the header only beside the secret.
import { checkBotId } from "botid/server";

/** The header value of BotID's verification. */
function verdict(verification) {
  if (verification.bypassed) {
    return "bypassed";
  }
  return verification.isHuman && !verification.isBot ? "human" : "bot";
}

export async function POST(request) {
  const verification = await checkBotId({
    advancedOptions: { headers: Object.fromEntries(request.headers) },
  });
  const answer = await fetch(`${process.env.PROVING_GROUND_ORIGIN}/botid/verify`, {
    method: "POST",
    headers: {
      "x-proving-ground-edge": process.env.PROVING_GROUND_EDGE_SECRET,
      "x-proving-ground-botid": verdict(verification),
    },
  });
  return new Response(await answer.text(), {
    status: answer.status,
    headers: { "content-type": "text/html; charset=utf-8" },
  });
}
