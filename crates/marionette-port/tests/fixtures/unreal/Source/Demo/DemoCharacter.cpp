#include "DemoCharacter.h"
#include "Components/BoxComponent.h"
#include "Kismet/GameplayStatics.h"

ADemoCharacter::ADemoCharacter()
{
	PrimaryActorTick.bCanEverTick = true;
	PickupBox = CreateDefaultSubobject<UBoxComponent>(TEXT("PickupBox"));
	PickupBox->SetupAttachment(RootComponent);
}

void ADemoCharacter::BeginPlay()
{
	Super::BeginPlay();
	PickupBox->OnComponentBeginOverlap.AddDynamic(this, &ADemoCharacter::OnPickupOverlap);
	GetWorld()->GetTimerManager().SetTimer(FireTimer, this, &ADemoCharacter::Fire, 0.5f, true);
}

void ADemoCharacter::Tick(float DeltaTime)
{
	Super::Tick(DeltaTime);
	FVector Loc = GetActorLocation();
	Loc.Z += FMath::Sin(GetWorld()->GetTimeSeconds()) * 10.f;
	SetActorLocation(Loc);
	UE_LOG(LogTemp, Log, TEXT("Ammo %d"), Ammo);
}

void ADemoCharacter::SetupPlayerInputComponent(UInputComponent* PlayerInputComponent)
{
	Super::SetupPlayerInputComponent(PlayerInputComponent);
	PlayerInputComponent->BindAxis("MoveForward", this, &ADemoCharacter::MoveForward);
	PlayerInputComponent->BindAction("Fire", IE_Pressed, this, &ADemoCharacter::Fire);
}

void ADemoCharacter::MoveForward(float Value)
{
	AddMovementInput(GetActorForwardVector(), Value);
}

void ADemoCharacter::Fire()
{
	if (Ammo <= 0) { return; }
	Ammo--;
	float Spread = FMath::RandRange(-1.f, 1.f);
	GetWorld()->SpawnActor<AActor>(GetClass(), GetActorLocation(), GetActorRotation());
}

void ADemoCharacter::OnPickupOverlap(UPrimitiveComponent* OverlappedComp, AActor* OtherActor, UPrimitiveComponent* OtherComp, int32 OtherBodyIndex, bool bFromSweep, const FHitResult& SweepResult)
{
	if (OtherActor && OtherActor->ActorHasTag("Pickup"))
	{
		OtherActor->Destroy();
		Ammo += 10;
	}
}
