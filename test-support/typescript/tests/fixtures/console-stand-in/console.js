// The stand-in Console's passkey ceremonies: registration, and the step-up a
// signed-in person confirms an action with. The server checks each one's
// challenge, type and origin.
const outcome = document.getElementById("passkey");

const bytes = (text) =>
  Uint8Array.from(atob(text.replaceAll("-", "+").replaceAll("_", "/")), (c) => c.charCodeAt(0));
const text = (buffer) =>
  btoa(String.fromCharCode(...new Uint8Array(buffer)))
    .replaceAll("+", "-")
    .replaceAll("/", "_")
    .replaceAll("=", "");

async function newChallenge() {
  const response = await fetch("/passkey/challenge", { method: "POST" });
  return response.json();
}

async function send(path, credential) {
  const response = await fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      id: credential.id,
      clientData: text(credential.response.clientDataJSON),
    }),
  });
  return (await response.json()).done;
}

document.getElementById("register").addEventListener("click", async () => {
  const { challenge: value, person } = await newChallenge();
  const credential = await navigator.credentials.create({
    publicKey: {
      challenge: bytes(value),
      rp: { name: "Goodfetch Console" },
      user: { id: new TextEncoder().encode(person), name: person, displayName: person },
      pubKeyCredParams: [{ type: "public-key", alg: -7 }],
      authenticatorSelection: { residentKey: "required", userVerification: "preferred" },
    },
  });
  outcome.textContent = (await send("/passkey/register", credential))
    ? "Passkey registered"
    : "Passkey refused";
});

document.getElementById("step-up").addEventListener("click", async () => {
  const { challenge: value } = await newChallenge();
  const credential = await navigator.credentials.get({
    publicKey: { challenge: bytes(value), userVerification: "preferred" },
  });
  outcome.textContent = (await send("/passkey/step-up", credential))
    ? "Confirmed with your passkey"
    : "Passkey refused";
});
