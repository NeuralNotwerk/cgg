from external_lib import Base


class P(Base):
    async def _ahook(self):
        return 1

    def _shook(self):
        return 2


async def free_async():
    return 3


def main():
    P()


main()
