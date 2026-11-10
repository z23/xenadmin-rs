#!/usr/bin/env node
// Make the lab's Xen Orchestra appliance manage the lab pool, so the recovery gate's "XO VM"
// is also the pool's manager. Credentials come from files, never from the command line:
//   XENADMIN_LAB_XO_FILE      one line "email password" for the appliance's admin account
//   XENADMIN_LAB_PASSWORD_FILE the lab-only root password of the pool coordinator
// Usage: node lab/xo_connect.mjs <xoa address> <coordinator address>
// On the appliance's factory account the password is rotated to the one in the XO file first.
import { readFileSync } from "node:fs";

const [xoa, coordinator] = process.argv.slice(2);
if (!xoa || !coordinator) {
  console.error("usage: xo_connect.mjs <xoa address> <coordinator address>");
  process.exit(2);
}
const [email, password] = readFileSync(process.env.XENADMIN_LAB_XO_FILE, "utf8").trim().split(/\s+/);
const rootPassword = readFileSync(process.env.XENADMIN_LAB_PASSWORD_FILE, "utf8").split("\n")[0].trim();
const FACTORY = { email: "admin@admin.net", password: "admin" };

const ws = new WebSocket(`${process.env.XENADMIN_LAB_XO_SCHEME ?? "wss"}://${xoa}/api/`);
let nextId = 1;
const pending = new Map();
ws.addEventListener("message", (event) => {
  const message = JSON.parse(event.data);
  if (message.id !== undefined && pending.has(message.id)) {
    const { resolve, reject } = pending.get(message.id);
    pending.delete(message.id);
    message.error ? reject(new Error(message.error.message || JSON.stringify(message.error))) : resolve(message.result);
  }
});
const call = (method, params = {}) =>
  new Promise((resolve, reject) => {
    const id = nextId++;
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
    setTimeout(() => pending.has(id) && (pending.delete(id), reject(new Error(`${method} timed out`))), 60000);
  });
const open = new Promise((resolve, reject) => {
  ws.addEventListener("open", resolve);
  ws.addEventListener("error", () => reject(new Error("websocket failed")));
});
await open;
let signedIn = false;
try {
  await call("session.signIn", { email, password });
  signedIn = true;
  console.log("signed in with the lab account");
} catch {
  await call("session.signIn", FACTORY);
  await call("user.changePassword", { oldPassword: FACTORY.password, newPassword: password });
  if (email !== FACTORY.email) {
    const me = await call("session.getUser");
    await call("user.set", { id: me.id, email });
  }
  console.log("factory account rotated to the lab account");
  signedIn = true;
}
if (!signedIn) process.exit(1);
const servers = await call("server.getAll");
let server = servers.find((s) => s.host === coordinator);
if (!server) {
  const id = await call("server.add", { host: coordinator, username: "root", password: rootPassword, allowUnauthorized: true, label: "xenadmin-rs lab" });
  console.log("pool server added", id);
} else {
  console.log("pool server already present", server.id, server.status);
}
for (let i = 0; i < 60; i++) {
  const all = await call("server.getAll");
  server = all.find((s) => s.host === coordinator);
  if (server && server.status === "connected") break;
  await new Promise((r) => setTimeout(r, 2000));
}
console.log("server status", server?.status, "pool", server?.poolNameLabel ?? "");
const objects = await call("xo.getAllObjects", { filter: { type: "VM" } });
console.log("VMs seen by XO:", Object.values(objects).map((v) => v.name_label).sort().join(", "));
ws.close();
