# witness-2

witness-2 is a second fork witness, next to witness-1.easybtx.com. It is the
same `btx-witness` program (crates/btx-core/src/bin/btx-witness.rs), running on
the api.btxscan.io box and reading that box's btxd2 node. Caddy on the box
serves it at:

    https://api.btxscan.io/witness/

It answers three read-only routes and nothing else:

    /witness/blocks/tip/height
    /witness/block-height/<h>
    /witness/signers/recent

Why: witness-1 is one host, and the easybtx.com census reads which keys sign
the chain from it alone. When it is down the census has no signer count at
all. With witness-2 the census asks both and uses whichever answers.

## Files

- `btx-witness-2.service`: the systemd unit for the box (User=azureuser,
  datadir /data/btx2, RPC 127.0.0.1:8434, listens on 127.0.0.1:3081 only).
- `install-witness-2.sh`: the one script that installs it. It carries the
  unit text inside itself, because the box only gets this one file.
- `test-install-witness-2.sh`: runs on any machine, never touches a box.
  Checks the unit text, the checksum check and the Caddyfile edit against
  `test-fixtures/Caddyfile.box-shape`, and with a Caddy that has the rate-limit
  plugin also starts it and proves `/witness/` reaches the witness while
  everything else still reaches electrs.

## Get the binary

Run the "Box tools (Linux)" workflow (.github/workflows/box-tools.yml) from
the Actions tab. Its artifact has `btx-witness` and `SHA256SUMS.txt`. The box
needs the binary at an https URL it can download, for example as a release
asset. Note its sha256 from `SHA256SUMS.txt`.

## The one command

From a machine with the Azure CLI, in this repo's folder:

    az vm run-command invoke -g RESOURCE_GROUP -n VM_NAME --command-id RunShellScript --scripts @deploy/esplora/witness-2/install-witness-2.sh --parameters "BINARY_URL" "SHA256"

It downloads the binary and refuses it unless the sha256 matches, installs
it, starts `btx-witness-2.service`, waits for a height on 127.0.0.1:3081,
then adds one `handle /witness/*` block to the api.btxscan.io site in
/etc/caddy/Caddyfile (after a timestamped backup), runs `caddy validate` and
reloads Caddy. If anything fails it puts everything back and says so. The
last line is the verdict.

Running it again is safe: the Caddyfile edit is skipped when the route is
already there.

## Check it

    curl https://api.btxscan.io/witness/blocks/tip/height

A block height means it works. Compare with
`curl https://witness-1.easybtx.com/blocks/tip/height`: the two should be
within a block or two.

## Undo

On the box, as root:

    systemctl disable --now btx-witness-2
    rm /etc/systemd/system/btx-witness-2.service /usr/local/bin/btx-witness
    systemctl daemon-reload
    cat /etc/caddy/Caddyfile.bak-witness2-STAMP > /etc/caddy/Caddyfile
    systemctl reload caddy

STAMP is the time in the backup's name (`ls /etc/caddy/Caddyfile.bak-witness2-*`).
`cat >` instead of `cp` keeps the file's owner. If Caddy was edited by hand
since, delete only the witness-2 block instead: it is the seven lines starting
with the `# witness-2:` comment inside the api.btxscan.io site.
