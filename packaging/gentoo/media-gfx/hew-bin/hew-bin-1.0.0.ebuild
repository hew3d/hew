# Copyright 2026 Kurt Granroth
# Distributed under the terms of the GNU General Public License v2

EAPI=8

inherit unpacker xdg

MY_PV="${PV}"
DESCRIPTION="3D modeler with SketchUp's interaction model on a solids-first kernel (binary)"
HOMEPAGE="https://hew3d.com https://github.com/hew3d/hew"
SRC_URI="
	amd64? ( https://github.com/hew3d/hew/releases/download/v${MY_PV}/Hew_${MY_PV}_amd64.deb -> ${P}_amd64.deb )
	arm64? ( https://github.com/hew3d/hew/releases/download/v${MY_PV}/Hew_${MY_PV}_arm64.deb -> ${P}_arm64.deb )
"
S="${WORKDIR}"

LICENSE="AGPL-3"
SLOT="0"
KEYWORDS="-* ~amd64 ~arm64"
RESTRICT="mirror strip"

RDEPEND="
	net-libs/webkit-gtk:4.1
	x11-libs/gtk+:3
"

QA_PREBUILT="*"

src_unpack() {
	unpack_deb ${A}
}

src_install() {
	# The .deb already carries the FHS layout: /usr/bin/hew, /usr/bin/hew-cli,
	# the desktop entry and hicolor icons.
	insinto /usr
	doins -r usr/*
	fperms 0755 /usr/bin/hew /usr/bin/hew-cli

	# Tell the app how it was installed (shells/tauri/src-tauri/src/
	# install_channel.rs). The .deb's binary carries the Deb marker, which
	# would otherwise send it to `dpkg`; Portage owns this copy, so the app
	# only announces updates and shows the emerge command.
	printf 'gentoo\n' > "${T}"/install-channel || die
	insinto /usr/share/hew
	doins "${T}"/install-channel
}
