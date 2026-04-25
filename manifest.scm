(use-modules (guix packages)
             (gnu)
             (gnu packages rust)
             (gnu packages nss)
             (gnu packages pkg-config)
             (gnu packages commencement)
             (gnu packages version-control))

(packages->manifest (list pkg-config
                          rust
                          (list rust "cargo")
                          (list rust "tools")
                          rust-analyzer
                          nss-certs
                          gcc-toolchain
                          git-minimal))
