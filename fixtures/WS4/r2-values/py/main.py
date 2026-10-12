from pkg import wif


def go():
    wif._refresher.stop()
    wif.start()
    wif.missing()
