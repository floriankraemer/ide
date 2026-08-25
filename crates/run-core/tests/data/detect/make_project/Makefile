.PHONY: build test clean

CFLAGS := -O2

build:
	gcc -o app main.c $(CFLAGS)

test: build
	./app --selftest

clean:
	rm -f app *.o

app.o: main.c
	gcc -c main.c

%.o: %.c
	gcc -c $<
