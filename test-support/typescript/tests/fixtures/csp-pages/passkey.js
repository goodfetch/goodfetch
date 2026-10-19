// Registers a passkey and signs in with it through navigator.credentials.
const result = document.getElementById("result");
const challenge = () => crypto.getRandomValues(new Uint8Array(32));

document.getElementById("register").addEventListener("click", async () => {
  try {
    const credential = await navigator.credentials.create({
      publicKey: {
        challenge: challenge(),
        rp: { name: "Fixture" },
        user: { id: new Uint8Array(16), name: "person", displayName: "Person" },
        pubKeyCredParams: [{ type: "public-key", alg: -7 }],
        authenticatorSelection: { residentKey: "required", userVerification: "preferred" },
      },
    });
    result.textContent = `registered ${credential.type}`;
  } catch (error) {
    result.textContent = `register failed: ${error.name}`;
  }
});

document.getElementById("sign-in").addEventListener("click", async () => {
  try {
    const credential = await navigator.credentials.get({
      publicKey: { challenge: challenge(), userVerification: "preferred" },
    });
    result.textContent = `signed in ${credential.type}`;
  } catch (error) {
    result.textContent = `sign-in failed: ${error.name}`;
  }
});
